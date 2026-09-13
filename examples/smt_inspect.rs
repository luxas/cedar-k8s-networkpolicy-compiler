//! Capture the SMT-LIB script the symbolic compiler sends to the solver, by
//! wrapping a `LocalSolver` in a solver that tees everything written to its
//! stdin into a buffer, then printing the transcript.
//!
//! Only what the *compiler* writes is seen: bytes the inner solver emits on its
//! own behalf — `enable_models` writing `produce-models`, `check_sat` writing
//! `(check-sat)` — go straight to its own writer without passing through the
//! wrapper, so they do not appear. A tee inside `LocalSolver` itself would
//! see them; this example makes do with the public `Solver` trait.
//!
//!     cargo run --example smt_inspect     # needs cvc5 on $PATH or in $CVC5

use cedar_policy::{Authorizer, Decision, PolicySet, Schema};
use cedar_policy_symcc::{
    CedarSymCompiler, CompiledPolicySet,
    solver::{DecisionWithModel, LocalSolver, Solver, SolverError},
};
use std::{io::Write, str::FromStr, sync::Arc};

use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

#[tokio::main]
async fn main() {
    // Parse Cedar schema
    let schema = Schema::from_cedarschema_str(
        r#"
        entity User;
        entity Document { owner: User };
        action view appliesTo {
            principal: [User],
            resource: [Document]
        };
    "#,
    )
    .unwrap()
    .0;

    // Parse Cedar policy set
    let policy_set = PolicySet::from_str(
        r#"
        permit(principal, action == Action::"view", resource)
        when { resource.owner == principal };
    "#,
    )
    .unwrap();

    // Initialize the symbolic compiler, teeing the script into one transcript.
    let cvc5 = LocalSolver::cvc5().unwrap();
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let buf2 = buf.clone();
    let wrapper = InspectSolver::new(
        cvc5,
        Box::new(move |written: &[u8]| {
            let _ = buf2.lock().unwrap().write_all(written);
        }),
    );
    let mut compiler = CedarSymCompiler::new(wrapper).unwrap();

    // Iterate through all request environments and check the property
    for req_env in schema.request_envs() {
        // Compile the policy for this request environment
        let compiled_policies = CompiledPolicySet::compile(&policy_set, &req_env, &schema).unwrap();

        // Verify that `policy_set` does not always allow any request
        let always_denies = compiler
            .check_always_allows_opt(&compiled_policies)
            .await
            .unwrap();
        assert!(!always_denies);

        // Similar to above, but returns a counterexample (a synthesized request
        // and entity store) which is denied by the policy set.
        let cex = compiler
            .check_always_allows_with_counterexample_opt(&compiled_policies)
            .await
            .unwrap()
            .unwrap();
        let resp = Authorizer::new().is_authorized(&cex.request, &policy_set, &cex.entities);
        assert!(resp.decision() == Decision::Deny);
    }

    println!(
        "{}",
        String::from_utf8(buf.lock().unwrap().to_vec()).unwrap()
    );
}

/// An inspection callback, boxed.
///
/// Boxed because closures have unnameable types and so cannot be a bare struct
/// field, and `Send` because `Solver`'s methods return
/// `impl Future<Output = _> + Send`.
type InspectFn = Box<dyn FnMut(&[u8]) + Send + 'static>;

/// An owned `AsyncWrite` that forwards straight through to the solver it owns.
///
/// The ownership is inverted on purpose: the writer owns the solver, not the
/// reverse. `Solver::smtlib_input` hands out a borrow that cannot be held
/// across calls — but a borrow created and dropped inside a single call is
/// free, and `poll_write` is a single call. So `Forwarder` owns `inner`
/// outright, and each poll asks it afresh for its writer, uses it, and lets
/// the borrow expire before returning. No buffer, no deferred flush, and the
/// callback sees exactly the bytes the writer accepted.
struct Forwarder<S> {
    /// The solver we forward to, which owns the real writer.
    inner: S,
    /// Called with each batch of bytes as it passes through. See [`InspectFn`].
    inspect: InspectFn,
}

impl<S: Solver + Unpin> tokio::io::AsyncWrite for Forwarder<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // `Pin::get_mut` is safe exactly because we require `Self: Unpin`.
        let me = self.get_mut();
        // The borrow lives for this expression and no longer. `Pin::new`
        // accepts it because the trait object carries `Unpin`.
        let res = Pin::new(me.inner.smtlib_input()).poll_write(cx, buf);
        // Report only bytes actually accepted, only on success.
        if let Poll::Ready(Ok(n)) = res {
            // `poll_write` guarantees `n <= buf.len()`, so this `get` always succeeds.
            if let Some(written) = buf.get(..n).filter(|w| !w.is_empty()) {
                (me.inspect)(written);
            }
        }
        res
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        Pin::new(me.inner.smtlib_input()).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        Pin::new(me.inner.smtlib_input()).poll_shutdown(cx)
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for Forwarder<S> {
    // Hand-written rather than derived: `dyn FnMut` is not `Debug`, so
    // `#[derive(Debug)]` would demand a bound the field can never satisfy.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Forwarder")
            .field("inner", &self.inner)
            .field("inspect", &"<closure>")
            .finish()
    }
}

/// A genuine wrapping solver, built around [`Forwarder`].
///
/// `InspectSolver` owns a `Forwarder`, which owns the inner solver. So
/// `smtlib_input()` is a borrow of our own field — the shape the trait
/// requires — and the `Solver` methods reach the inner solver by going
/// *through* the forwarder to the solver it owns.
#[derive(Debug)]
struct InspectSolver<S> {
    /// Owns the inner solver; see [`Forwarder`] for why the nesting runs this way.
    forwarder: Forwarder<S>,
}

impl<S: Solver + Send + Unpin> InspectSolver<S> {
    fn new(inner: S, inspect: InspectFn) -> Self {
        Self {
            forwarder: Forwarder { inner, inspect },
        }
    }
}

impl<S: Solver + Send + Unpin> Solver for InspectSolver<S> {
    fn smtlib_input(&mut self) -> &mut (dyn tokio::io::AsyncWrite + Unpin + Send) {
        // A borrow of our own field, unsize-coerced on the way out.
        &mut self.forwarder
    }

    async fn enable_models(&mut self) -> Result<(), SolverError> {
        self.forwarder.inner.enable_models().await
    }

    async fn check_sat(&mut self) -> Result<cedar_policy_symcc::solver::Decision, SolverError> {
        self.forwarder.inner.check_sat().await
    }

    async fn check_sat_with_model(&mut self) -> Result<DecisionWithModel, SolverError> {
        self.forwarder.inner.check_sat_with_model().await
    }
}
