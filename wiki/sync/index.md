---
type: index
summary: "Sync and storage, the sync state, the Hub server, its deployment and the Android agent through the Hub."
parent: "[[index]]"
status: current
---

# Sync and Hub

These pages describe how devices share a vault: the local-first sync model, the built sync, the Hub server and how it is deployed, and the Android agent that reaches the vault through the Hub.

<!-- wiki:children -->
- [Sync and storage](sync-and-storage.md): Local-first sync with CRDTs, attachments by hash, page history and cold history storage.
- [Sync (state)](sync-state.md): The built sync (vault_sync + core hooks): hub server, binary protocol, token pairing, delta sync by version vectors, conflicts, purges, lazy blobs, numbers.
- [Hub server (state)](hub.md): notes-hub, the server hub: sync over axum behind Caddy, device tokens from an Authentik (OIDC) login, the agent at /agent/ws (JSON-RPC over WebSocket); config, routes, protocol, security.
- [Hub deployment](hub-deploy.md): Local amd64 container build and deploy kit; Authentik, the existing Caddy network, provider sign-ins, backup and rollback.
- [Android agent through the Hub](mobile-agent.md): Hub-only mobile agent, authenticated vault sync, local acceptance tests and the phone sign-in steps.
<!-- /wiki:children -->
