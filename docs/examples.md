# prism executable examples

These examples are intentionally stable and are executed by `tests/docs_examples.rs`.
They complement the larger reference examples in `README.md`, `SPEC.md`, and `docs/cli.md`.

```prism-example
$ prism dt --utc --from 0 --fmt %Y-%m-%dT%H:%M:%SZ
> 1970-01-01T00:00:00Z
```

```prism-example
$ prism seq 1..3 --fmt item-%03d
> item-001
> item-002
> item-003
```

```prism-example
$ prism case snake FooBar
> foo_bar
```

```prism-example
$ prism slug --sep _ "Hello, World!"
> hello_world
```

```prism-example
$ prism field 2..-1 --osep ,
< a b c d
> b,c,d
```

```prism-example
$ prism enc hex
< abc
> 6162630a
```

```prism-example
$ prism hash sha256
< abc
> edeaaff3f1774ad2888673770c6d64097e391bc362d7d6fb34982ddf0efd18cb
```

```prism-example
$ prism do "trim | case snake | slug"
<   Hello World
> hello-world
```
