# Quickstart

Five minutes from a directory of YAML to a connectivity verdict. This page covers
building the tool, the two compile steps, and the first query; the
[guides](README.md#topics) take each feature further.

## Prerequisites

- **Rust** (a stable toolchain; the crate uses edition 2024) and, for the symbolic
  features (`--pod-cidr`, `connect`), **cvc5** on `$PATH` or pointed at by `$CVC5`
  (`brew install cvc5`, or a [release binary](https://github.com/cvc5/cvc5/releases)).
- **Network access for the first build.** The symbolic evaluator and cedar-woodpecker
  are unpublished, so `Cargo.toml` fetches them from a pinned commit of the
  [cedar-woodpecker](https://github.com/luxas/cedar-woodpecker) fork (see
  [architecture](contributing/architecture.md#the-cedar-fork) for why, and how the pin is
  bumped).

```sh
cargo build            # the np2cedar binary, with the Kubernetes client
cargo build --no-default-features   # offline compiler only — no HTTP/TLS stack
```

## Compile, then ask

The repo ships a worked example, an Istio-Bookinfo-shaped set of pods and one
NetworkPolicy:

```sh
np2cedar compile   -f examples/bookinfo -o policies.cedar
np2cedar entities  -f examples/bookinfo -o entities.json
np2cedar check     --policies policies.cedar --entities entities.json \
                   --from default/productpage --to default/details --port 9080
```

```
ingress  default/productpage -> default/details  TCP:9080  ALLOW  (default/details-allow-productpage ingress[0])
egress   default/productpage -> default/details  TCP:9080  ALLOW  (catch-all egress (not isolated by any NetworkPolicy))
=> ALLOWED
```

Both directions must allow — Kubernetes requires the source's egress rules *and* the
destination's ingress rules to agree, and the parenthesized reasons name the Kubernetes
object (or the default-allow catch-all) that decided each
([why two directions](concepts/encoding.md)).

The intermediate files are optional: `check`, `reachable` and `connect` all accept `-f`
directly, compiling on the fly, and `--cluster` reads a live API server instead of files
([live cluster](guides/live-cluster.md)).

## Your first honest UNKNOWN

`np2cedar entities` never invents an address: a pod without `status.podIPs` has an
**unknown** address, and questions that depend on it come back `UNKNOWN` with the residual
condition that names what is missing ([why that is the correct answer, not a cop-out](concepts/partial-evaluation.md)):

```sh
np2cedar check -f tests/data/unknown-pod-ip --from default/gateway --to default/mystery --port 443
```

```
note: no status.podIPs for default/mystery; their addresses are left unknown, so ipBlock rules over them evaluate to a residual rather than a decision
ingress  default/gateway -> default/mystery  TCP:443  ALLOW  (catch-all ingress (not isolated by any NetworkPolicy))
egress   default/gateway -> default/mystery  TCP:443  UNKNOWN  (default/egress-to-internet egress[0])
    @k8sDirection("egress")
    @k8sPolicy("default/egress-to-internet")
    @k8sRule("0")
    permit(
      principal,
      action,
      resource
    ) when {
      IpAddr::"default/mystery".addr.isInRange(ip("0.0.0.0/0")) && (!IpAddr::"default/mystery".addr.isInRange(ip("10.0.0.0/8")))
    };
=> UNKNOWN (see the residuals above)
```

The address is unknown but not arbitrary — it lies in the cluster's pod CIDR — and saying
so collapses the verdict:

```sh
np2cedar check -f tests/data/unknown-pod-ip --from default/gateway --to default/mystery \
               --port 443 --pod-cidr 10.244.0.0/16
```

```
note: no status.podIPs for default/mystery; their addresses are left unknown, so ipBlock rules over them evaluate to a residual rather than a decision
note: default/egress-to-internet egress[0] can never match a pod address in 10.244.0.0/16
ingress  default/gateway -> default/mystery  TCP:443  ALLOW  (catch-all ingress (not isolated by any NetworkPolicy))
egress   default/gateway -> default/mystery  TCP:443  DENY  (default/egress-to-internet egress[0], no residual rule can match a pod address in 10.244.0.0/16)
=> DENIED (blocked on egress)
```

That is the symbolic evaluator at work — [the guide](guides/discharging-unknowns.md) shows
the allow and stays-unknown cases too, and [the deep dive](concepts/symbolic-discharge.md)
explains why the collapse is sound.

## Where next

- Deciding connections and reading verdicts: [checking connectivity](guides/checking-connectivity.md)
- What a pod can reach, as residual policies: [reachability](guides/reachability.md)
- Every way any pod can reach any pod, synthesized: [connect](guides/connect.md)
- All commands and flags: [CLI reference](reference/cli.md)
