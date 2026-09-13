# Plan: validate the static analysis against Cilium, empirically

## Context

Everything that checks np2cedar's correctness so far is self-referential: truth tables
derived by hand from the API contract, a hand-written Cedar translation by the same
hands, and an assertion that the API server's defaulting matches ours. All of it tests
"does the tool agree with a reading of the spec" — none of it tests "does the tool agree
with what a real CNI does to real packets."

This feature adds that. A kind cluster running **Cilium** enforces a fixture's
NetworkPolicies for real; **kubesonde** injects nmap ephemeral containers and probes every
pod pair, reporting Allow/Deny per (source, destination, port); and a new test asserts
np2cedar's verdict matches the observed one for every probe. A disagreement means one of
the three is wrong, and that is worth knowing.

### What this does and does not establish

- It validates against **Cilium's** enforcement — one implementation, not the
  specification. A mismatch could be our bug, kubesonde's, or Cilium's: investigate, do
  not suppress.
- Only **TCP**, **pod to pod**, **within one namespace** (a Kubesonde object probes
  exactly one), and only on ports the destination genuinely serves.
- nmap cannot distinguish "policy dropped it" from "nothing is listening". The fixture
  guarantees a live listener on every declared port, which is what makes a `Deny` mean
  policy.

## The e2e cluster

A **second** kind cluster, `np2cedar-e2e`, because Cilium needs `disableDefaultCNI: true`
and the integration cluster must stay fast. Cilium is installed with the `cilium` CLI (one
binary, fetched by `hack/lib.sh` like `kind`), then `cilium status --wait`. Its kubeconfig
is `.kube/e2e-config`, separate from the integration cluster's.

kubesonde is installed from its pinned release manifest, with the controller image pinned
to the same tag — upstream references `:latest`, which would make runs irreproducible.
Results come from an HTTP endpoint on the controller (`/probes` on port 2709), not from
the CR's status, and probing is asynchronous with no completion signal: the script
port-forwards, then polls until the item count is non-empty and unchanged across
consecutive polls, and writes `.e2e/probes.json`.

kubesonde's monitor image is published for amd64 only, which would wedge probing on
arm64 because both injected containers must reach `Running`. Its command is overridden to
`sh` and the process it would run is exec'd separately with its failure tolerated, so
`spec.monitorImage: alpine:3` clears the gate and loses only netstat-derived *extra*
probes, which this comparison does not use: the probe set comes from declared
`containerPort`s.

## Fixture (`tests/data/e2e/manifests.yaml`)

One namespace, four pods (`web`, `api`, `cache`, `lonely`) each running
`agnhost netexec --http-port=8080` and declaring `containerPort: 8080`, and three
policies: `api-allow-web` (ingress to `api` from `web` on 8080), `cache-deny-all`
(ingress isolation with no rules) and `cache-egress` (egress from `cache` to `lonely`
only). The twelve ordered pairs on port 8080 give a deliberate mix, including a Deny
caused by *egress* rather than ingress — which is what actually exercises the two-action
conjunction against a real dataplane:

| → | web | api | cache | lonely |
| --- | --- | --- | --- | --- |
| **web** | – | Allow | Deny | Allow |
| **api** | Allow | – | Deny | Allow |
| **cache** | Deny (egress) | Deny (both) | – | Allow |
| **lonely** | Allow | Deny | Deny | – |

## The test (`tests/e2e.rs`, feature `e2e`)

Gated on the feature, and like the live-cluster suite it **fails rather than skips** when
its inputs are missing.

1. Read `.e2e/probes.json` (override with `$E2E_PROBES`).
2. Build np2cedar's model by fetching from the e2e cluster with `cluster::fetch`, so the
   live path is exercised too rather than re-reading the YAML.
3. Keep only comparable probes: `type == "Probe"`, Pod to Pod, both in the fixture
   namespace, TCP only, source ≠ destination, and only ports the destination declares.
4. Compare each with the `check` path (`eval::evaluate` over both directions, combined).

Three tests, because one of them is about not fooling ourselves: the observed matrix must
match the fixture's hand-derived table; every probe must agree with np2cedar (an
`Unknown` is a failure — the fixture has every address known and no `ipBlock`); and the
probe set must be non-empty, cover all twelve pairs and contain both outcomes, so a
kubesonde that returned nothing cannot pass silently.

## Scripts and CI

| Script | Does |
| --- | --- |
| `hack/e2e-up.sh` | ensure `kind` and the `cilium` CLI; create `np2cedar-e2e` from `hack/kind-cilium.yaml`; install and wait for Cilium; install kubesonde (pinned); apply the fixture and wait for the pods; apply the Kubesonde object; poll `/probes` to stability; write `.e2e/probes.json`. Idempotent. |
| `hack/e2e-down.sh` | delete the cluster, remove the kubeconfig and the probe results |
| `hack/e2e-test.sh` | `e2e-up.sh`, `cargo test --features e2e`, teardown unless `KEEP_CLUSTER=1` |

`Cargo.toml` gains `e2e = ["cluster"]`; `.gitignore` gains `/.e2e/`; `Verdict` gains the
derives the comparison needs to sort and dedup observations. CI gains a third job, `e2e`,
calling the same scripts, with a failure step that dumps Cilium status, the Kubesonde
object, the controller logs and `probes.json`.

## Verification

```sh
hack/e2e-up.sh                  # slow the first time: Cilium and image pulls
KUBECONFIG=.kube/e2e-config cargo test --features e2e
hack/e2e-test.sh                # the whole cycle from nothing
cargo test                      # unchanged, must stay green
```

The fixture's observed matrix must match the hand-derived table above. If Cilium
disagrees with the table, the table is the thing to re-derive — that is the entire point
of the exercise. A deliberately broken run (a NetworkPolicy deleted after probing) must
make the comparison fail and name the pair, confirming it is actually comparing.
