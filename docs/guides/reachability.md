# What can a pod reach: `np2cedar reachable`

`reachable` asks the open-ended question: from this pod, to **any** pod, what does the
policy set say? The destination is left symbolic, and the answer is a set of residual
policies per direction — conditions over the peer that would make the connection allowed.

```sh
np2cedar reachable (-f <files> | --cluster | --policies P --entities E) \
                   --from ns/pod [--port N] [--protocol TCP] [--pod-cidr CIDR]...
```

Omitting `--port` leaves the port unknown too, so the residuals also describe which ports
qualify.

## Reading residuals

Each printed residual is a real Cedar policy, annotated with the Kubernetes object it came
from, partially evaluated against everything that *is* known. A residual like

```cedar
@k8sPolicy("default/egress-to-internet")
permit(...) when {
  resource.ip.addr.isInRange(ip("0.0.0.0/0")) && (!resource.ip.addr.isInRange(ip("10.0.0.0/8")))
};
```

reads: "this pod may egress to any peer whose address is outside `10.0.0.0/8`". The
conditions are over the symbolic peer (`resource`) — its labels, namespace, address, and
the port. How partial evaluation produces these is covered in
[the deep dive](../concepts/partial-evaluation.md).

## Pruning with the pod CIDR

With [`--pod-cidr`](discharging-unknowns.md), residuals that cannot match any **pod** peer
are pruned and named, and a direction whose residuals all vanish collapses to a definite
verdict (real output, `tests/data/unknown-pod-ip`):

```
# ingress from default/gateway to any Pod: ALLOW
# egress from default/gateway to any Pod: DENY
# pruned by --pod-cidr (can never match a Pod peer): default/egress-to-internet egress[0]
```

The gateway pod's only egress rule allows the public internet — so against *pods*, it
allows nothing at all.

Neighbours: [checking connectivity](checking-connectivity.md) ·
[discharging unknowns](discharging-unknowns.md) ·
[CLI reference](../reference/cli.md#reachable)
