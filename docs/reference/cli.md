# CLI reference

```
np2cedar compile   -f <path|dir|->...  [-o policies.cedar]
np2cedar entities  -f <path|dir|->...  [-o entities.json]
np2cedar check     (-f ... | --policies P --entities E)
                   --from ns/pod --to ns/pod|IP --port N [--protocol TCP]
np2cedar reachable (-f ... | --policies P --entities E)
                   --from ns/pod [--port N] [--protocol TCP]
np2cedar schema    [-o -]
```

Exactly one source must be named per invocation: `-f`, or the
`--policies`/`--entities` pair.

## Sources

- **`-f`** takes files, directories (recursed over `*.yaml`/`*.yml`/`*.json` in sorted
  order) or `-` for stdin. Multi-document YAML is split and dispatched on `kind`, so
  NetworkPolicies, Pods and Namespaces may share a file; `*List` wrappers are unwrapped.
- **`--policies P --entities E`** replays files produced by `compile` and `entities`;
  the two flags come as a pair.

Entity UIDs are namespaced paths — `Pod::"default/productpage"`,
`Namespace::"default"` — so `--from ns/pod` addresses them directly and residual policies
stay readable.

## `compile`, `entities`, `schema`

Produce the Cedar policy set, the entity-store JSON, and the bundled Cedar schema
respectively; `-o` writes to a file (`-` for stdout). Every compiled policy set has
already passed strict validation.

## `check`

Decides one connection. `--from` is a `namespace/name` pod; `--to` is a pod or a bare IP
(which becomes a concrete cluster-external `IpEndpoint`); `--port` is required and
`--protocol` defaults to `TCP`. Output: one line per direction with the verdict and its
reasons, residual policies under any `UNKNOWN`, then the combined verdict.

| Exit code | Meaning |
| --- | --- |
| 0 | `ALLOWED` — both directions allow |
| 1 | `DENIED` — at least one direction denies |
| 2 | `UNKNOWN` — undecidable from what was given |
| 3 | error |

See [checking connectivity](../guides/checking-connectivity.md).

## `reachable`

The destination is left symbolic: prints, per direction, the verdict against "any Pod"
and the residual policies describing which peers qualify. Omitting `--port` leaves the
port symbolic too. Always exits 0 (or 3 on error) — the output is the answer. See
[reachability](../guides/reachability.md).

Neighbours: [quickstart](../quickstart.md) · [schema reference](schema.md)
