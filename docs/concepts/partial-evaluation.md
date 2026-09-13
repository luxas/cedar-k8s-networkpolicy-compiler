# Honest unknowns: type-aware partial evaluation

Why `check` can answer `UNKNOWN`, why that is the correct answer rather than a
limitation, and the encoding trick that makes it possible. The guide-level view is in
[checking connectivity](../guides/checking-connectivity.md); what to *do* about unknowns
is [discharging unknowns](../guides/discharging-unknowns.md).

## Never invent an address

`np2cedar entities` records a pod's address from `status.podIPs` when it is there — and
leaves it **unknown** when it is not. An early design that assigned synthetic addresses
from a `--pod-cidr` was explicitly rejected: an invented address makes `ipBlock`
questions come back confidently and wrong. The principle that replaced it: if a rule
admits `10.0.0.0/8` and the peer's address was never given, whether the connection is
allowed *genuinely depends on something we do not know* — and the tool should say
exactly that.

## The all-or-nothing constraint, and the `IpAddr` side entity

Cedar's partial entities are all-or-nothing per entity: when attributes are supplied,
every required attribute must be present. A Pod therefore cannot have known labels and an
unknown address. The encoding's answer is to give the address its own entity —
`Pod.ip: IpAddr`, `IpAddr = { addr: ipaddr }` — so the Pod stays fully concrete and only
its `IpAddr` is unknown. In the entity JSON, unknown is simply an `IpAddr` entity with no
`attrs` key.

(A consequence used later: an attribute-less `IpAddr` entity is *always* an unknown pod
address — external `IpEndpoint`s are always written with their concrete address — which
is exactly the set of entities [symbolic discharge](symbolic-discharge.md) attaches its
CIDR assumptions to.)

## Everything goes through TPE

Because any pod's address may be unknown, **every** evaluation path uses Cedar's
type-aware partial evaluation, plain `check` included. Per direction:

| | |
| --- | --- |
| `ALLOW` / `DENY` | decided from what the store knows |
| `UNKNOWN` | some policy could not be decided; its **residual** — the policy partially evaluated down to exactly the unknown parts — is printed |

A residual is precise about what is missing:

```
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
```

Everything known has been folded away — the selectors matched, the port matched — and
what remains is the one fact the cluster never stated, named by entity
(`IpAddr::"default/mystery"`).

In `reachable`, the same machinery runs with the whole peer symbolic: the residuals *are*
the answer, conditions over which pods (and ports) qualify.

## From honest to decided

An unknown address is still constrained — it lies in the pod CIDR — and handing that
constraint to the symbolic evaluator collapses most unknowns exactly:
[symbolic discharge](symbolic-discharge.md).

Neighbours: [the encoding](encoding.md) ·
[discharging unknowns](../guides/discharging-unknowns.md) ·
[symbolic discharge](symbolic-discharge.md)
