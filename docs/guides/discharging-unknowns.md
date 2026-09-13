# Discharging unknowns: `--pod-cidr`

An unknown pod address is not arbitrary: it lies in the cluster's pod CIDR. The
repeatable `--pod-cidr` flag (on `check`, `reachable` and `connect`) hands exactly that
constraint to a symbolic evaluator backed by cvc5, which decides, per residual rule,
whether it can still match — pruning the ones that cannot, and collapsing the verdict
when that settles a direction. The machinery and its soundness argument live in
[the deep dive](../concepts/symbolic-discharge.md).

Requires cvc5 on `$PATH` or `$CVC5` pointing at the binary. Without residuals to
discharge, no solver is even spawned.

## The three outcomes, on one fixture

`tests/data/unknown-pod-ip`: pod `gateway` is egress-isolated by one rule allowing
`0.0.0.0/0 except 10.0.0.0/8`; pod `mystery` has no recorded address.

**The CIDR sits inside the exception — DENY.** No pod address can match the rule:

```sh
np2cedar check -f tests/data/unknown-pod-ip --from default/gateway --to default/mystery \
               --port 443 --pod-cidr 10.244.0.0/16
# note: default/egress-to-internet egress[0] can never match a pod address in 10.244.0.0/16
# => DENIED (blocked on egress)          exit code 1
```

**The CIDR avoids the exception — ALLOW.** Every address the pod could have matches:

```sh
... --pod-cidr 172.16.0.0/12
# => ALLOWED                             exit code 0
```

**The CIDR straddles the exception — still UNKNOWN.** `10.0.0.0/7` covers addresses
inside and outside `10.0.0.0/8`, so the answer genuinely depends on where the address
falls, and the tool says so:

```sh
... --pod-cidr 10.0.0.0/7
# => UNKNOWN                             exit code 2
```

That last case is the point: the evaluator is **exact, not heuristic**. A collapse only
happens when it holds for every address the constraint permits.

## Dual-stack

Repeat the flag: `--pod-cidr 10.244.0.0/16 --pod-cidr fd00::/8`. Note that Cedar's
`isInRange` across address families is `false`, matching Kubernetes — a v6 pod address
does not match a v4 `0.0.0.0/0` — so on the fixture above, dual-stack still denies.

Neighbours: [checking connectivity](checking-connectivity.md) ·
[symbolic discharge](../concepts/symbolic-discharge.md) ·
[connect](connect.md) (the same flag prunes the synthesis)
