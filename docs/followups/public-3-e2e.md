# Follow-ups: public-3-e2e

## PR description

Check the static analysis against a real dataplane. `hack/e2e-test.sh` stands up a kind
cluster with Cilium as its CNI, applies a four-pod fixture whose pods all serve TCP 8080,
lets [kubesonde](https://github.com/kubesonde/kubesonde) probe every pod pair with nmap,
and then runs `cargo test --features e2e`, which asserts that np2cedar's verdict matches
what Cilium actually did for every probe:

```sh
hack/e2e-up.sh                                    # cluster, Cilium, kubesonde, fixture, probes
KUBECONFIG=.kube/e2e-config cargo test --features e2e
```

```
running 3 tests
test the_probe_set_is_worth_comparing_against ... ok
test the_observed_matrix_is_the_one_the_fixture_was_designed_for ... ok
test every_probe_agrees_with_the_static_analysis ... ok
```

Before this, every check of the compiler's correctness was self-referential — truth
tables and a hand translation by the same hands as the compiler. Now the tool agrees with
packets on all twelve ordered pod pairs, including the denial caused by *egress* rather
than ingress, the case that exercises the two-action conjunction against a dataplane
rather than against the author's reasoning. The test says plainly what that does not
establish: one CNI, TCP only, one namespace, and only where a listener exists.

## Review findings

1. **The e2e job runs on every pull request.** It takes many minutes and pulls large
   images; if it proves flaky it should move to a schedule or a label-gated trigger,
   which is a decision to make with data from a few runs.
2. **The settle heuristic can fire early.** `hack/e2e-up.sh` declares the results settled
   when the item count is unchanged across two consecutive polls (twenty seconds). On one
   run on a laptop, kubesonde paused at 34 items long enough to pass that gate and then
   went on to 36, so the written `probes.json` lacked the `cache -> api` pair and two of
   the three tests failed with "was never probed"; re-fetching `/probes` a minute later
   gave the complete set and all three passed. Make the gate stricter: require the count
   to be stable for longer, and — since the fixture is known — keep polling until every
   ordered pair on the declared port has been probed. Comparing a hash of the sorted
   items rather than the count would also catch a `resultingAction` that flips.
3. **The served-port filter hides a class of disagreement.** Probes against ports the
   destination does not declare are dropped, which is right for the nmap confound, but
   a policy allowing an undeclared port can never be observed. A second fixture pod
   listening on a port it does not declare would let the test say whether kubesonde's
   declared-port probe set is the only gap.
4. **`model()` fetches the cluster once per test**, three times in total. Cheap here,
   but a `OnceLock` around the fetched `Loaded` would also make the three tests judge
   the same snapshot.
5. **The `alpine:3` monitor-image workaround is undocumented for users.** It lives in a
   comment in `hack/lib.sh`; the testing page should mention it, since anyone running
   the e2e on arm64 will otherwise wonder why the monitor is not the upstream image.
6. **Tool versions are pinned in `hack/lib.sh` only.** `KIND_NODE_IMAGE`, Cilium and
   kubesonde versions should be visible in the testing page, and a comment should tie
   the node image to the `k8s-openapi` API version feature.
7. **kubesonde's `errors` array is read but never asserted on.** A run with probe errors
   still passes if enough probes succeeded; at least print them, or fail when they
   concern fixture pods.
