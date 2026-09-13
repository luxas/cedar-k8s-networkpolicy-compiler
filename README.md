# cedar-k8s-networkpolicy-compiler

This is a proof of concept, building on top of the experimental [`cedar-woodpecker`] project.
See that project's README for more details on the rationale and methods.

The high-level idea of this project is to showcase how Cedar can function as an intermediate representation for access control, by "compiling" various Kubernetes `NetworkPolicy` formats into a Cedar-compatible representation which can be analyzed and reasoned about using generic tools, like [Cedar Analysis] and [`cedar-woodpecker`]. For now, I started with just compiling core Kubernetes `NetworkPolicy` objects, but I hope most or all features of similar concepts in Cilium, Calico, Istio, Linkerd could be compiled into this intermediate Cedar form as well (that's what we'll find out through this project!)

It is worth pointing out that the author of the ideas is Lucas Käldström (@luxas), the ideas are **NOT** LLM-generated, LLMs have just been used as a tool to show something concrete (during the so far short time between Aug 25 - Sept 12 I've had developing these ideas). I've also discussed this with Jacopo Bufalino, which has provided some feedback. It should also be mentioned that this `README.md` document is completely written by me, and I consider this document to be the primary contribution in this repo. Eventually, I intend to improve (most likely by starting more or less from scratch with regards to every feature PR) the code, docs, tests and (hopefully in the future) proofs, if the ideas make sense to the cloud native community. I'm happy to upstream the features to some CNCF project as well, as wanted/needed.

My main goal at this moment is to:

1. validate the ideas by discussing with others (I'm not 100% certain of whether all parts of the ideas make sense)
1. ask for feedback on the direction by the community (feel free to email me about this, or send a DM on the CNCF Slack or LinkedIn!)
1. showcase in hopefully a little bit more detail what it means for Cedar to be "analyzable" and/or have a "decidable encoding into mathematical logic", and what the benefits and opportunities are

> **WARNING:** The commits in this repo **MUST NOT be used in production**; they serve only as a concretization of the ideas presented, to evaluate what direction to evolve the ideas towards, and to give something that can be experimentally tested and iterated on, in search of the final form of the features.

The implementation's own documentation (quickstart, guides, deep dives and reference) starts at [`docs/README.md`](docs/README.md).

In the rest of this text, I'll summarize the ideas in a blog post-style way initially, then go more technical.
For now, I've only had time to go one pass over this text, I'll try to make it more understandable over time, if needed.
I intend to turn this text into a series of "real" blog posts later, most likely going into more depth on each topic there.

[`cedar-woodpecker`]: https://github.com/luxas/cedar-woodpecker
[Cedar Analysis]: https://cedarpolicy.com/blog/introducing-cedar-analysis

## Why?

[Kubernetes NetworkPolicies] allows a user to restrict the network flow in a Kubernetes cluster. By default, any workload can talk to any other workload and the internet in both directions (ingress/egress). But not only does there exist "standard" `NetworkPolicy` objects, there exists many other extension flavours with similar but slightly different semantics. In order for workload A to talk to workload B, both ingress for `A->B` and egress for `A->B` needs to be allowed. Furthermore, workloads/Pods are selected by labels.

Reasoning about the cross-product of what are the actual combinations that survive the `AND` (indeed, some cross-terms might be contradictory and get pruned out), how various APIs interact, when a Pod is non-isolated (the default) because it's not selected by any NetworkPolicy, whether some NetworkPolicies are redundant (as there is a larger, shadowing policy) is no easy feat for administrators. Hopefully, some automated reasoning tricks could help here.

However, the problems here are generic and fairly involved to solve correctly. Thus, instead of writing a NetworkPolicy-specific analyzer (and effectively solver), let's try to build on top of existing technology. Cedar, which encodes its policies into [`SMT-LIB`] format for generic solvers like `cvc5` (even with proofs of correctness for increased certainty) could thus get us quite far. Remaining is thus a way to faithfully translate (or "compile") the `NetworkPolicy` API semantics into Cedar policies.

[Kubernetes NetworkPolicies]: https://kubernetes.io/docs/concepts/services-networking/network-policies/
[`SMT-LIB`]: https://smt-lib.org/

## What

The implementation is divided into roughly 5 parts:

### Cedar policy synthesis from core `NetworkPolicy` objects

This is the primary "compilation" step. There are two separate actions: `ingress` and `egress` for each direction, and two types of `principal`/`resource` types: Pods and arbitrary IP addresses (which might be inside or outside of the cluster, as determined by a Pod CIDR range optionally specified).

Already from this step, one can:

1. check whether a certain concrete Pod A can perform `ingress`/`egress` to another certain concrete Pod B
1. use Cedar's [Typed Partial Evaluation] to ask "what are the possible other Pods that Pod A is allowed to send `egress` traffic to?".

However, at this point, neither query answers whether traffic actually can flow between A and B, as that requires both ingress and egress permissions, and the Cedar policies are only concerned with one direction at a time (for now).

However, one can already use [Cedar Analysis] to check whether some `NetworkPolicy` is redundant as it is shadowed by another. Recall that NetworkPolicies are allow-only, and if one policy is strictly bigger, the smaller one could be removed without any change to the network flows.

Finally, the compiler needs to add the non-isolation default, by building a predicate which allows ingress/egress for such Pods that otherwise aren't matched by any NetworkPolicy.

[Typed Partial Evaluation]: https://cedarpolicy.com/blog/tpe

Right now, only core Kubernetes NetworkPolicies are compiled, but that is planned to be extended in the future, which would also add L7 reasoning support.

### Query actual workload state from the cluster

The Cedar encoding offers both a way to reason about the policies symbolically, as well as concretely. In the concrete case, one needs to get the information about what Pods actually exist, and what labels they have. This can either be sourced from static files or a live cluster, and allows building a matrix of who should be able to talk to who.

### End to end testing with `kubesonde`

However, how do we know the compilation into Cedar is correct? We should ensure that what the Cedar authorizer predicts should happen is actually what happens in a Cluster. For this, [`kubesonde`] is a handy tool, with the following description in its own README:

> Kubesonde is a tool that probes and visualizes the actual network connectivity of applications running in a Kubernetes cluster, so you can compare it against the network policies you meant to enforce. It works by instrumenting live pods at runtime and reporting every connection attempt observed, rather than relying only on what NetworkPolicy manifests declare.
>
> Kubesonde is **not** a network policy engine, admission controller, or firewall: it does not block or modify traffic, and it does not replace tools like Cilium, Calico, or Kubernetes NetworkPolicies. It is a diagnostic and auditing tool for finding gaps between the connectivity you think you have and the connectivity you actually have.

[`kubesonde`]: https://github.com/kubesonde/kubesonde

Thus are end to end tests set up to differentially test the prediction against the ground truth.

### Symbolically reason about unknown Pod IPs

When reasoning about "what other Pods is Pod A allowed to send `egress` traffic to?" one can get false positives, if there is a NetworkPolicy which allows A to perform egress to an IP subnet. If that IP subnet intersects with the Pod subnet, it might genuinely be that A might be able to send some traffic to another target Pod, if the target Pod randomly gets assigned such a compatible IP at runtime. However, if the IP range A is allowed to send traffic to does _not_ intersect with the Pod subnet, that can never happen, and thus such a "possible policy" be pruned from the reachability query output.

This is exactly what the Symbolic Cedar Evaluator in the [`cedar-woodpecker`] repo; read more about that feature there.

### Synthesizing compound policies for action `connect` = `ingress` AND `egress`

As previously mentioned, the Cedar policies are written only for one direction at a time, mirroring how the NetworkPolicies are structured. However, what administrators ultimately are interested in is "who can talk to who?". To answer this question, we use [`cedar-woodpecker`]'s Symbolic Cedar Evaluator to prune never-true crossterm policies (see the Policy Simplification chapter in that README if you're interested in how it works in practice), and [`cedar-woodpecker`] itself to synthesize normalized policies which describe the exact conditions under which connections are possible.

The output is in [disjunctive normal form] (OR of ANDs), which indeed can be exponentionally large in comparison to the original policy. See this example:

```
(a OR b OR c) AND 
(d OR e OR f)
```

reduces to a "simplified" version of

```
(a AND d) OR 
(a AND e) OR 
(a AND f) OR 
(b AND d) OR 
(b AND e) OR 
(b AND f) OR 
(c AND d) OR 
(c AND e) OR 
(c AND f)
```

[disjunctive normal form]: https://en.wikipedia.org/wiki/Disjunctive_normal_form

This example demonstrates the `3^2` or `k^n` exponential blowup, which shows why it's so hard for human administrators to keep all those possible combinations in their head, and why the symbolic evaluator is so crucial in pruning impossible combinations for a cleaner and non-redundant output.

## Future work

On top of the future work items mentioned already in this README, it'd be interesting to perform multiple reachability steps, e.g. in order to prove that some classes of workloads in a parameterized set `Src = {w \in Workloads | P_src(w)}` can never (even in theory) send a request which transitively would fan out new requests into workloads in the `Dest = {w \in Workloads | P_dest(w)}` set. This could show (probably together with a couple of other things) two environments are properly isolated from each other.

Those "other thing" would probably mean integrating this project with [`kubernetes-cedar-authorizer`] (which will be donated to the Cedar project) and `nodeSelector` etc. to show that privilege escalation cannot either happen through e.g. breaking out of a container on a certain node and moving laterally that way.

[`kubernetes-cedar-authorizer`]: https://github.com/upbound/kubernetes-cedar-authorizer

## Copyright

Lucas Käldström

## License

Apache 2.0
