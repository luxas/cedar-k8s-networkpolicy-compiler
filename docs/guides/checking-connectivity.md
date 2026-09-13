# Checking connectivity: `np2cedar check`

`check` decides one connection — source pod, destination pod or IP, port, protocol — by
evaluating **both** directions and ANDing them, exactly as Kubernetes does. This page is
about running it and reading its output; [the encoding](../concepts/encoding.md) explains
why there are two directions, and [partial evaluation](../concepts/partial-evaluation.md)
explains the third verdict.

```sh
np2cedar check (-f <files> | --policies P --entities E) \
               --from ns/pod --to ns/pod|IP --port N [--protocol TCP]
```

## Reading the output

```
ingress  default/productpage -> default/details  TCP:9080  ALLOW  (default/details-allow-productpage ingress[0])
egress   default/productpage -> default/details  TCP:9080  ALLOW  (catch-all egress (not isolated by any NetworkPolicy))
=> ALLOWED
```

One line per direction: the verdict, then in parentheses *where it came from* — a
NetworkPolicy and rule index (`default/details-allow-productpage ingress[0]`), or the
catch-all that carries Kubernetes' default-allow for pods no policy of that direction
selects.

| Direction verdict | Meaning |
| --- | --- |
| `ALLOW` / `DENY` | decided from what the store knows |
| `UNKNOWN` | genuinely undecidable — the residual policy printed under the line says what is missing |

Combined: `ALLOWED` iff both directions allow, `DENIED` if either denies, else `UNKNOWN`.
Exit codes: `0`, `1`, `2` respectively, `3` for an error — so `check` works in scripts.

## Destinations

`--to` takes a `namespace/name` pod, or a **bare IP address**, which becomes a
cluster-external `IpEndpoint` with that concrete address — useful for "may this pod reach
8.8.8.8?". NetworkPolicy governs pods only, so an external endpoint is never *isolated*;
what can restrict the connection is the source's egress rules.

## When the verdict is UNKNOWN

An `UNKNOWN` names the pod whose address the cluster never recorded and prints the exact
residual condition. The way forward is to give the store the address (`status.podIPs` in
your manifests); the residual tells you which rule turns on it.

Neighbours: [reachability](reachability.md) ·
[partial evaluation](../concepts/partial-evaluation.md) ·
[CLI reference](../reference/cli.md#check)
