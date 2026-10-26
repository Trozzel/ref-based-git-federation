# ref-based-fed

A proof of concept for **Git federation with local-first authority** using **Git refs**, **Git bundles**, and **NATS**.

This project explores a simple federation model for multiple sites where:

- each site owns its own authoritative repository state
- remote sites keep awareness copies of other sites
- remote sites can propose changes to other sites
- the owning site accepts or rejects those proposals
- accepted changes are announced so other sites can fetch and synchronize

The goal is to support **DDIL-friendly**, **no-quorum**, **no-master/slave** federation using mostly standard Git behavior.

---

## Why this exists

Traditional Git hosting and replication patterns often assume one of these:

- a primary/secondary relationship
- a single canonical upstream
- continuous connectivity
- centralized forge workflows

This project is aimed at a different model:

- **local-first authority**
- **site autonomy**
- **operation while isolated**
- **awareness of remote differences**
- **proposal-based cross-site collaboration**

In this design, a site is authoritative for its own repository, while other sites may track it, inspect it, and propose changes to it.

---

## Core ideas

### 1. Local-first authority

Each site owns its own repo.

Example:

- Site 1 is authoritative for `repo/1`
- Site 2 is authoritative for `repo/2`
- Site 3 is authoritative for `repo/3`

Other sites may keep remote copies of those repos, but they do not override the owning site.

### 2. Git refs as federation state

Instead of requiring a full forge from the start, this POC models federation with Git refs.

Examples:

- authoritative refs
- published refs
- tracking refs
- inbox/proposal refs

This makes it possible to separate:

- accepted local state
- remote observed state
- inbound proposals

### 3. NATS for signaling

NATS is used as the event bus for federation notifications.

Typical events include:

- proposal created
- proposal received
- proposal accepted
- proposal rejected
- site sync available

NATS is not intended to be the source of truth for Git content. Git remains the source of truth.

### 4. Git bundles for disconnected proposal transfer

For DDIL and intermittently connected scenarios, remote proposal changes can be packaged as a Git bundle and transferred to the owning site.

The owning site can then:

- verify the bundle
- import it into an inbox ref
- review it
- accept or reject it

### 5. Fetch for synchronization

When a site accepts a change into its authoritative branch, other sites can fetch that updated ref after receiving a federation event.

---

## Target user stories

### User story 1: site changes its own repo

- Site 1 creates a local change in `repo/1`
- Site 1 reviews and approves the change
- Site 1 merges the change into its authoritative branch
- Site 1 publishes a NATS notification
- Other sites fetch the accepted change from Site 1

### User story 2: remote site proposes change to another site

- Site 2 checks out its copy of `repo/1`
- Site 2 makes changes intended for Site 1
- Site 2 creates a Git bundle containing the proposal branch
- Site 2 sends proposal metadata over NATS
- Site 1 receives and imports the proposal into an inbox ref
- Site 1 reviews and accepts or rejects the proposal
- If accepted, Site 1 merges it into its authoritative branch
- Site 1 notifies other sites via NATS
- Other sites fetch the accepted update

---

## What this project is not

At this stage, this project is **not** trying to be:

- a full forge
- a replacement for GitHub/GitLab/Forgejo
- a globally writable multi-master branch system
- a quorum-based distributed consensus system

This is a **federation workflow POC**, focused on:

- refs
- bundles
- synchronization
- proposals
- local authority

---

## Current direction

The current implementation direction is:

- **Rust** for the agent and workflow logic
- standard **Git** behavior for repository operations
- **NATS** for event distribution
- proposal/import/accept flows built around refs and bundles

The initial focus is to prove the federation mechanics first, before introducing a forge or UI.

---

## Planned repository layout

```text
src/
  git/
    mod.rs
    core.rs
docs/
  federation-flow.md
Cargo.toml
```

## Frontend

The proposal review UI lives in `frontend/`.

Run the Axum server on its default port, then start the frontend:

```bash
cd frontend
npm install
npm run dev
```

Vite serves the React app at `http://127.0.0.1:5173` and proxies `/api` requests to `http://127.0.0.1:8081`.

To point the UI at a different Axum server port:

```bash
cd frontend
npm run dev -- --api-port=8082
npm run build -- --api-port=8082
```

You can also use environment variables:

```bash
VITE_API_PORT=8083 npm run dev
VITE_API_PORT=8083 npm run build
VITE_API_BASE_URL=http://127.0.0.1:8083 npm run build
```

## Additional docs

- `docs/federation-flow.md` describes the broader federation workflow
- `docs/following.md` describes the `.federation/following` policy file and how
  it affects advertisements, updates, and remote reconciliation
