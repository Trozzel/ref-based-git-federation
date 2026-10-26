# Federation Flow: Leveraging Git Refs

## Federation Flow Diagram

```mermaid
flowchart TD
    subgraph A["Publishing Site (Site A)"]
        LC["1. Local change received or created"]
        C["2. Commit change"]
        P["3. Update site-owned published ref<br/>refs/heads/site-a/published"]
        LC --> C --> P
    end

    N["4. Announce event on NATS<br/>origin_site, repo, ref, commit"]
    G["5. Peers fetch published ref from Site A Git/Gitea"]
    
    subgraph B["Receiving Site (Site B)"]
        I["6. Import into tracking / inbox ref"]
        V["7. Validate<br/>trust, base commit, policy, schema"]
        D{"8. Promote locally?"}
        A1["9a. Promote to integration / active branch"]
        A2["9b. Keep as inbox ref / request catch-up / reject"]
        I --> V --> D
        D -->|Yes| A1
        D -->|No| A2
    end

    P --> N
    N --> G
    G --> I
```

## User stories

### User Story 1: Site 1 changes its own repo

- User at Site 1 creates branch in local repo/1
- Opens PR / MR at Site 1
- Gets approval
- Merges into authoritative repo/1
- Federation agent republishes accepted change
- Site 2 and Site 3 update their shadow repo/1
- Remote sites become up to date for awareness and future proposals

#### Sequence diagram for User Story 1

```mermaid
sequenceDiagram
    participant User1 as Site 1 User
    participant Site1 as Site 1 Git/Gitea
    participant FedAgent as Federation Agent
    participant Site2 as Site 2 Git/Gitea
    participant Site3 as Site 3 Git/Gitea
    User1->>Site1: Create branch and open PR/MR
    Site1->>User1: PR/MR created, awaiting approval
    User1->>Site1: Get approval and merge PR/MR
    Site1->>FedAgent: Notify of accepted change
    FedAgent->>Site1: Republish accepted change to refs/heads/site-1/published
    FedAgent->>Site2: Notify of new published ref
    FedAgent->>Site3: Notify of new published ref
    Site2->>FedAgent: Fetch published ref and update shadow repo/1
    Site3->>FedAgent: Fetch published ref and update shadow repo/1
```

### User Story 2: Site 1 proposes change to Site 2

- Site 1 user works in a local workspace derived from Site 2’s current shadow repo/2
- Site 1 opens a remote proposal targeting Site 2’s authoritative repo/2
- Federation agent transfers proposal branch/ref and metadata to Site 2
- Site 2 sees it in native Forgejo UX
- Site 2 admin/reviewer approves and merges locally
- Site 2’s accepted merge becomes authoritative
- Resulting commit is synchronized back to Site 1 shadow repo/2
- Originating proposal is marked merged/closed with link to authoritative merge

#### Sequence diagram for User Story 2

```mermaid
sequenceDiagram
    participant User1 as Site 1 User
    participant Site1 as Site 1 Git/Gitea
    participant FedAgent as Federation Agent
    participant Site2 as Site 2 Git/Gitea
    User1->>Site1: Create proposal branch targeting Site 2
    Site1->>FedAgent: Notify of new proposal with metadata
    FedAgent->>Site2: Transfer proposal branch/ref and metadata
    Site2->>FedAgent: Acknowledge receipt of proposal
    Site2->>Site2: Display proposal in Forgejo UX
    Site2->>Site2: Admin/reviewer approves and merges proposal
    Site2->>FedAgent: Notify of accepted merge and new authoritative commit
    FedAgent->>Site1: Synchronize new authoritative commit to shadow repo/2
    Site1->>FedAgent: Mark originating proposal as merged/closed with link to authoritative merge
```
