# graphnight

Python bindings for [GraphNight](https://github.com/niranjanaryan/graphnight) — an embeddable semantic layer for AI agents and humans.

**Status: beta.** Auth, caching, and Docker exist on the Rust server; these bindings are a local YAML/SQL client surface and still evolving.

## Install

```bash
pip install graphnight
```

## Quick start

```python
from graphnight import GraphNightClient

client = GraphNightClient(storage_path="./graphnight_data")
print(client.list_models())
```

Scaffold data with the CLI from the [GitHub repo](https://github.com/niranjanaryan/graphnight):

```bash
cargo install --path crates/graphnight-cli
graphnight init ./my-project
```

## Links

- Docs: https://github.com/niranjanaryan/graphnight/tree/main/docs
- Changelog: https://github.com/niranjanaryan/graphnight/blob/main/CHANGELOG.md
- Security: https://github.com/niranjanaryan/graphnight/blob/main/SECURITY.md

## License

Apache-2.0
