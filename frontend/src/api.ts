export type Proposal = {
  target_site: string;
  source_site: string;
  proposal_id: string;
};

export type SerializedProposalState =
  | "Queued"
  | "Backlog"
  | "Ready"
  | "Accepted"
  | "Resolved"
  | "Rejected"
  | { Conflicted: string[] }
  | { Error: string };

export type SerializedProposalRecord =
  | SerializedProposalState
  | {
      state: SerializedProposalState;
      commit_hash: string;
      base_commit_hash?: string;
    }
  | [SerializedProposalState, string];

export type ProposalStatusTuple = [Proposal, SerializedProposalRecord];

export type ProposalStateKind =
  | "Queued"
  | "Backlog"
  | "Ready"
  | "Accepted"
  | "Resolved"
  | "Rejected"
  | "Conflicted"
  | "Error";

export type ProposalStateView = {
  kind: ProposalStateKind;
  files?: string[];
  message?: string;
  inferred?: boolean;
};

export type ProposalDirection = "inbound" | "outbound";

export type ProposalRow = {
  proposal: Proposal;
  state: ProposalStateView;
  commitHash: string;
  baseCommitHash: string;
  hasLocalRef: boolean;
  hasRemoteRef: boolean;
  direction: ProposalDirection;
};

export type ProposalConflictFile = {
  file: string;
  ours: string | null;
  theirs: string | null;
  base: string | null;
};

export type FederationSitePreference = {
  site: string;
  followed: boolean;
  auto_accept_fast_forward_merge: boolean;
};

export type FederationSitePreferencesResponse = {
  follow_all: boolean;
  sites: FederationSitePreference[];
};

export type SiteInfo = {
  site_name: string;
};

export type ConflictResolutionChoice = "ours" | "theirs";

type ProposalActionResponse = {
  ok: boolean;
  message: string;
  proposal_ref?: string | null;
};

const apiBaseUrl = __API_BASE_URL__.replace(/\/$/, "");
const apiUrl = (path: string): string => `${apiBaseUrl}${path}`;

export const proposalKey = (proposal: Proposal): string =>
  `${proposal.target_site}:${proposal.source_site}:${proposal.proposal_id}`;

export const proposalRowKey = (row: ProposalRow): string =>
  `${row.direction}:${proposalKey(row.proposal)}`;

export const proposalLabel = (proposal: Proposal): string =>
  `${proposal.source_site}/${proposal.proposal_id}`;

export function normalizeState(
  state: SerializedProposalState,
  inferred = false
): ProposalStateView {
  if (typeof state === "string") {
    return { kind: state, inferred };
  }

  if ("Conflicted" in state) {
    return { kind: "Conflicted", files: state.Conflicted, inferred };
  }

  if ("Error" in state) {
    return { kind: "Error", message: state.Error, inferred };
  }

  return { kind: "Error", message: "Unexpected proposal state payload", inferred };
}

export function normalizeRecord(record: SerializedProposalRecord): {
  state: ProposalStateView;
  commitHash: string;
  baseCommitHash: string;
} {
  if (Array.isArray(record)) {
    const [state, commitHash] = record;
    return { state: normalizeState(state), commitHash, baseCommitHash: "" };
  }

  if (typeof record === "object" && "state" in record) {
    return {
      state: normalizeState(record.state),
      commitHash: record.commit_hash,
      baseCommitHash: record.base_commit_hash ?? ""
    };
  }

  return {
    state: normalizeState(record),
    commitHash: "",
    baseCommitHash: ""
  };
}

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiUrl(path), {
    headers: { "Content-Type": "application/json" },
    ...init
  });
  const contentType = response.headers.get("content-type") ?? "";
  const body = contentType.includes("application/json")
    ? await response.json()
    : await response.text();

  if (!response.ok) {
    const message = typeof body === "string" ? body : JSON.stringify(body);
    throw new Error(message || `${response.status} ${response.statusText}`);
  }

  return body as T;
}

async function fetchText(path: string): Promise<string> {
  const response = await fetch(apiUrl(path));
  const body = await response.text();
  if (!response.ok) {
    throw new Error(body || `${response.status} ${response.statusText}`);
  }
  return body;
}

function mergeInboundProposalRows(
  statuses: ProposalStatusTuple[],
  localProposals: Proposal[],
  remoteProposals: Proposal[]
): ProposalRow[] {
  const localKeys = new Set(localProposals.map(proposalKey));
  const remoteKeys = new Set(remoteProposals.map(proposalKey));
  const rows = new Map<string, ProposalRow>();

  for (const [proposal, record] of statuses) {
    const key = proposalKey(proposal);
    const { state, commitHash, baseCommitHash } = normalizeRecord(record);
    rows.set(key, {
      proposal,
      state,
      commitHash,
      baseCommitHash,
      hasLocalRef: localKeys.has(key),
      hasRemoteRef: remoteKeys.has(key),
      direction: "inbound"
    });
  }

  for (const proposal of localProposals) {
    const key = proposalKey(proposal);
    if (!rows.has(key)) {
      rows.set(key, {
        proposal,
        state: normalizeState("Ready", true),
        commitHash: "",
        baseCommitHash: "",
        hasLocalRef: true,
        hasRemoteRef: remoteKeys.has(key),
        direction: "inbound"
      });
    }
  }

  for (const proposal of remoteProposals) {
    const key = proposalKey(proposal);
    if (!rows.has(key)) {
      rows.set(key, {
        proposal,
        state: normalizeState("Backlog", true),
        commitHash: "",
        baseCommitHash: "",
        hasLocalRef: false,
        hasRemoteRef: true,
        direction: "inbound"
      });
    }
  }

  return sortProposalRows(Array.from(rows.values()));
}

function mergeOutboundProposalRows(
  statuses: ProposalStatusTuple[],
  sourceProposals: Proposal[]
): ProposalRow[] {
  const sourceKeys = new Set(sourceProposals.map(proposalKey));
  const rows = new Map<string, ProposalRow>();

  for (const [proposal, record] of statuses) {
    const key = proposalKey(proposal);
    const { state, commitHash, baseCommitHash } = normalizeRecord(record);
    rows.set(key, {
      proposal,
      state,
      commitHash,
      baseCommitHash,
      hasLocalRef: false,
      hasRemoteRef: false,
      direction: "outbound"
    });
  }

  for (const proposal of sourceProposals) {
    const key = proposalKey(proposal);
    if (!rows.has(key)) {
      rows.set(key, {
        proposal,
        state: normalizeState("Queued", true),
        commitHash: "",
        baseCommitHash: "",
        hasLocalRef: false,
        hasRemoteRef: false,
        direction: "outbound"
      });
    } else {
      const existing = rows.get(key)!;
      rows.set(key, {
        ...existing,
        hasLocalRef: sourceKeys.has(key)
      });
    }
  }

  return sortProposalRows(Array.from(rows.values()));
}

function sortProposalRows(rows: ProposalRow[]): ProposalRow[] {
  return rows.sort((a, b) => {
    const target = a.proposal.target_site.localeCompare(b.proposal.target_site);
    if (target !== 0) return target;
    const source = a.proposal.source_site.localeCompare(b.proposal.source_site);
    if (source !== 0) return source;
    return a.proposal.proposal_id.localeCompare(b.proposal.proposal_id);
  });
}

export async function listProposalRows(): Promise<ProposalRow[]> {
  const [
    statuses,
    localProposals,
    remoteProposals,
    sourceStatuses,
    sourceProposals
  ] = await Promise.all([
    fetchJson<ProposalStatusTuple[]>("/api/proposals/statuses"),
    fetchJson<Proposal[]>("/api/proposals/active"),
    fetchJson<Proposal[]>("/api/proposals/remote"),
    fetchJson<ProposalStatusTuple[]>("/api/proposals/source/statuses"),
    fetchJson<Proposal[]>("/api/proposals/source")
  ]);

  return [
    ...mergeInboundProposalRows(statuses, localProposals, remoteProposals),
    ...mergeOutboundProposalRows(sourceStatuses, sourceProposals)
  ];
}

export function getProposalDiff(proposal: Proposal): Promise<string> {
  return fetchText(
    `/api/proposals/${encodeURIComponent(proposal.source_site)}/${encodeURIComponent(
      proposal.proposal_id
    )}/diff`
  );
}

export function getProposalRowDiff(row: ProposalRow): Promise<string> {
  if (row.direction === "outbound") {
    return fetchText(
      `/api/proposals/source/${encodeURIComponent(row.proposal.target_site)}/${encodeURIComponent(
        row.proposal.proposal_id
      )}/diff`
    );
  }

  return getProposalDiff(row.proposal);
}

export function reconcileProposals(): Promise<string> {
  return fetchJson<string>("/api/proposals/reconcile", { method: "POST" });
}

export function listFederationSites(): Promise<FederationSitePreferencesResponse> {
  return fetchJson<FederationSitePreferencesResponse>("/api/federation/sites");
}

export function getSiteInfo(): Promise<SiteInfo> {
  return fetchJson<SiteInfo>("/api/site");
}

export function addFollowingSite(site: string): Promise<FederationSitePreferencesResponse> {
  return fetchJson<FederationSitePreferencesResponse>("/api/federation/following", {
    method: "POST",
    body: JSON.stringify({ site })
  });
}

export function setAllowFastForwardMerge(
  site: string,
  enabled: boolean
): Promise<FederationSitePreferencesResponse> {
  return fetchJson<FederationSitePreferencesResponse>(
    "/api/federation/allow-fast-forward-merge",
    {
      method: "POST",
      body: JSON.stringify({ site, enabled })
    }
  );
}

export function getProposalConflictFile(
  proposal: Proposal,
  filePath: string
): Promise<ProposalConflictFile> {
  return fetchJson<ProposalConflictFile>(
    `/api/proposals/conflicts/${encodeURIComponent(proposal.source_site)}/${encodeURIComponent(
      proposal.proposal_id
    )}/${filePath
      .split("/")
      .map((segment) => encodeURIComponent(segment))
      .join("/")}`
  );
}

export function resolveProposalConflicts(
  proposal: Proposal,
  resolutions: Array<{ file: string; choice: ConflictResolutionChoice }>
): Promise<ProposalActionResponse> {
  return fetchJson<ProposalActionResponse>("/api/proposals/conflicts/resolve", {
    method: "POST",
    body: JSON.stringify({
      source_site: proposal.source_site,
      proposal_id: proposal.proposal_id,
      resolutions
    })
  });
}

export function acceptProposal(
  proposal: Proposal
): Promise<ProposalActionResponse> {
  return fetchJson<ProposalActionResponse>("/api/proposals/accept", {
    method: "POST",
    body: JSON.stringify({
      source_site: proposal.source_site,
      proposal_id: proposal.proposal_id
    })
  });
}

export function rejectProposal(
  proposal: Proposal
): Promise<ProposalActionResponse> {
  return fetchJson<ProposalActionResponse>("/api/proposals/reject", {
    method: "POST",
    body: JSON.stringify({
      source_site: proposal.source_site,
      proposal_id: proposal.proposal_id
    })
  });
}

export function resolveProposal(
  proposal: Proposal
): Promise<ProposalActionResponse> {
  return fetchJson<ProposalActionResponse>("/api/proposals/resolve", {
    method: "POST",
    body: JSON.stringify({
      source_site: proposal.source_site,
      proposal_id: proposal.proposal_id
    })
  });
}
