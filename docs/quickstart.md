# Quickstart

Five minutes from a directory of YAML to a connectivity verdict. This page covers
building the tool, the two compile steps, and the first query; the
[guides](README.md#topics) take each feature further.

## Prerequisites

- **Rust** (a stable toolchain; the crate uses edition 2024).

```sh
cargo build            # the np2cedar binary
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

The intermediate files are optional: `check` and `reachable` both accept `-f` directly,
compiling on the fly.

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
      ((IpAddr::"default/mystery".addr).isInRange(ip("0.0.0.0/0"))) && (!((IpAddr::"default/mystery".addr).isInRange(ip("10.0.0.0/8"))))
    };
=> UNKNOWN (see the residuals above)
```

Everything known has been folded away — the selectors matched, the port matched — and
what remains is the one fact the cluster never stated, named by entity. Give the store
the address (`status.podIPs` in the manifest) and the verdict is decided.

## Where next

- Deciding connections and reading verdicts: [checking connectivity](guides/checking-connectivity.md)
- What a pod can reach, as residual policies: [reachability](guides/reachability.md)
- All commands and flags: [CLI reference](reference/cli.md)
