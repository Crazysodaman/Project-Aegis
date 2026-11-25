# Aegis IPC contracts v1

These JSON Schemas mirror the Rust types in `crates/aegis-contracts`.

- Contracts are authority-free.
- The owner token is carried in the request, but only the Safe boundary verifies it.
- `request.schema.json` and `response.schema.json` define the local framing contract.
- Status and STOP payloads are versioned separately.
- A breaking change requires a new version directory and protocol version.

## Core contracts v1

`contracts/core/v1` mirrors the Wave 1b Core identity, initialization, integrity, and registry contracts. They are authority-free and do not grant Core activation or lifecycle authority.
