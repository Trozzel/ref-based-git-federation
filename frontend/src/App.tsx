import { useCallback, useEffect, useMemo, useState } from "react";
import {
  acceptProposal,
  addFollowingSite,
  getSiteInfo,
  getProposalConflictFile,
  getProposalRowDiff,
  listFederationSites,
  listProposalRows,
  proposalLabel,
  proposalRowKey,
  reconcileProposals,
  rejectProposal,
  resolveProposalConflicts,
  resolveProposal,
  setAllowFastForwardMerge,
  type ConflictResolutionChoice,
  type FederationSitePreference,
  type ProposalConflictFile,
  type ProposalRow,
  type ProposalStateKind,
  type ProposalStateView
} from "./api";

type Toast = {
  tone: "info" | "success" | "danger";
  message: string;
};

const terminalStates = new Set<ProposalStateKind>(["Accepted", "Resolved", "Rejected"]);

function IconRefresh() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M20 6v6h-6" />
      <path d="M4 18v-6h6" />
      <path d="M18.5 9A7 7 0 0 0 6.8 6.8L4 9.5" />
      <path d="M5.5 15A7 7 0 0 0 17.2 17.2L20 14.5" />
    </svg>
  );
}

function IconCheck() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="m5 13 4 4L19 7" />
    </svg>
  );
}

function IconX() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 6 18 18" />
      <path d="M18 6 6 18" />
    </svg>
  );
}

function IconResolve() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3a9 9 0 1 0 9 9" />
      <path d="M12 8v5l3 3" />
    </svg>
  );
}

function IconBranch() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M7 5v9a4 4 0 0 0 4 4h6" />
      <path d="M17 14v8" />
      <path d="M17 2v6" />
      <path d="M4 5a3 3 0 1 0 6 0 3 3 0 0 0-6 0Z" />
      <path d="M14 5a3 3 0 1 0 6 0 3 3 0 0 0-6 0Z" />
      <path d="M14 18a3 3 0 1 0 6 0 3 3 0 0 0-6 0Z" />
    </svg>
  );
}

function stateText(state: ProposalStateView): string {
  if (state.kind === "Conflicted" && state.files?.length) {
    return `Conflicted (${state.files.length})`;
  }
  return state.kind;
}

function StateBadge({ state }: { state: ProposalStateView }) {
  return (
    <span className={`state-badge state-${state.kind.toLowerCase()}`}>
      {stateText(state)}
    </span>
  );
}

function ProposalListItem({
  row,
  selected,
  onSelect
}: {
  row: ProposalRow;
  selected: boolean;
  onSelect: (row: ProposalRow) => void;
}) {
  const title =
    row.direction === "outbound"
      ? `${row.proposal.target_site}/${row.proposal.proposal_id}`
      : proposalLabel(row.proposal);

  return (
    <button
      className={`proposal-card ${selected ? "selected" : ""}`}
      type="button"
      onClick={() => onSelect(row)}
    >
      <span className="proposal-main">
        <span className="proposal-title">{title}</span>
        <span className="proposal-meta">
          {row.proposal.source_site} to {row.proposal.target_site}
        </span>
      </span>
      <StateBadge state={row.state} />
    </button>
  );
}

function DiffView({ diff, loading, error }: { diff: string; loading: boolean; error: string }) {
  const lines = useMemo(() => diff.split("\n"), [diff]);

  if (loading) {
    return <div className="diff-empty">Loading diff</div>;
  }

  if (error) {
    return <div className="diff-error">{error}</div>;
  }

  if (!diff.trim()) {
    return <div className="diff-empty">No file changes</div>;
  }

  return (
    <pre className="diff-view" aria-label="Proposal diff">
      {lines.map((line, index) => {
        const kind =
          line.startsWith("+") && !line.startsWith("+++")
            ? "add"
            : line.startsWith("-") && !line.startsWith("---")
              ? "remove"
              : line.startsWith("@@")
                ? "hunk"
                : line.startsWith("diff --git")
                  ? "file"
                  : "context";
        return (
          <code className={`diff-line diff-${kind}`} key={`${index}-${line}`}>
            <span className="diff-line-number">{index + 1}</span>
            <span className="diff-line-text">{line || " "}</span>
          </code>
        );
      })}
    </pre>
  );
}

function ConflictPane({
  title,
  content
}: {
  title: string;
  content: string | null;
}) {
  return (
    <section className="conflict-pane" aria-label={title}>
      <header className="conflict-pane-header">{title}</header>
      <pre className="conflict-pane-body">{content ?? "File does not exist on this side"}</pre>
    </section>
  );
}

function ConflictInspector({
  files,
  selectedFile,
  onSelectFile,
  selections,
  onSelectChoice,
  onApplySelections,
  detail,
  loading,
  error
}: {
  files: string[];
  selectedFile: string;
  onSelectFile: (file: string) => void;
  selections: Record<string, ConflictResolutionChoice>;
  onSelectChoice: (file: string, choice: ConflictResolutionChoice) => void;
  onApplySelections: () => void;
  detail: ProposalConflictFile | null;
  loading: boolean;
  error: string;
}) {
  const selectedChoice = selections[selectedFile];
  const allFilesSelected = files.length > 0 && files.every((file) => selections[file]);

  return (
    <section className="conflict-inspector" aria-label="Conflict inspector">
      <div className="conflict-file-list">
        <span className="detail-label">Conflicted Files</span>
        <div className="conflict-file-buttons">
          {files.map((file) => (
            <button
              key={file}
              className={`conflict-file-button ${selectedFile === file ? "selected" : ""}`}
              type="button"
              onClick={() => onSelectFile(file)}
            >
              {file}
              {selections[file] ? ` (${selections[file]})` : ""}
            </button>
          ))}
        </div>
      </div>

      <div className="conflict-choice-bar">
        <button
          className={`icon-button secondary ${selectedChoice === "ours" ? "selected-choice" : ""}`}
          type="button"
          onClick={() => onSelectChoice(selectedFile, "ours")}
          disabled={!selectedFile}
        >
          Keep Ours
        </button>
        <button
          className={`icon-button secondary ${selectedChoice === "theirs" ? "selected-choice" : ""}`}
          type="button"
          onClick={() => onSelectChoice(selectedFile, "theirs")}
          disabled={!selectedFile}
        >
          Keep Theirs
        </button>
        <button
          className="icon-button primary"
          type="button"
          onClick={onApplySelections}
          disabled={!allFilesSelected}
        >
          Apply Selections
        </button>
      </div>

      {loading ? <div className="diff-empty">Loading conflict</div> : null}
      {!loading && error ? <div className="diff-error">{error}</div> : null}
      {!loading && !error && detail ? (
        <div className="conflict-panes">
          <ConflictPane title="Ours (main)" content={detail.ours} />
          <ConflictPane title="Theirs (proposal)" content={detail.theirs} />
          <ConflictPane title="Base" content={detail.base} />
        </div>
      ) : null}
    </section>
  );
}

function SelectedDetails({ row }: { row: ProposalRow }) {
  const files = row.state.kind === "Conflicted" ? row.state.files ?? [] : [];
  return (
    <div className="detail-grid" aria-label="Proposal details">
      <div>
        <span className="detail-label">Direction</span>
        <strong>{row.direction === "outbound" ? "Created here" : "Targeting this site"}</strong>
      </div>
      <div>
        <span className="detail-label">Target</span>
        <strong>{row.proposal.target_site}</strong>
      </div>
      <div>
        <span className="detail-label">Source</span>
        <strong>{row.proposal.source_site}</strong>
      </div>
      <div>
        <span className="detail-label">Proposal</span>
        <strong>{row.proposal.proposal_id}</strong>
      </div>
      <div>
        <span className="detail-label">State</span>
        <StateBadge state={row.state} />
      </div>
      {row.commitHash ? (
        <div className="commit-detail">
          <span className="detail-label">Commit</span>
          <code>{row.commitHash}</code>
        </div>
      ) : null}
      {files.length > 0 ? (
        <div className="conflict-list">
          <span className="detail-label">Conflicts</span>
          <ul>
            {files.map((file) => (
              <li key={file}>{file}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {row.state.kind === "Error" && row.state.message ? (
        <div className="state-message">
          <span className="detail-label">Error</span>
          <p>{row.state.message}</p>
        </div>
      ) : null}
    </div>
  );
}

function FederationSettings({
  sites,
  followAll,
  newSite,
  onNewSiteChange,
  onAddSite,
  onToggleAllowFfm,
  adding,
  updatingSite
}: {
  sites: FederationSitePreference[];
  followAll: boolean;
  newSite: string;
  onNewSiteChange: (value: string) => void;
  onAddSite: () => void;
  onToggleAllowFfm: (site: string, enabled: boolean) => void;
  adding: boolean;
  updatingSite: string | null;
}) {
  const followedSites = sites.filter((site) => site.followed);

  return (
    <section className="federation-settings" aria-label="Federation settings">
      <div className="settings-header">
        <div>
          <span className="detail-label">Federation</span>
          <strong>Followed Sites</strong>
        </div>
        {followAll ? <span className="settings-pill">Following all discovered sites</span> : null}
      </div>

      <div className="settings-site-list">
        {followedSites.length === 0 ? (
          <div className="settings-empty">No followed sites configured</div>
        ) : (
          followedSites.map((site) => (
            <label className="site-setting-row" key={site.site}>
              <span className="site-setting-name">{site.site}</span>
              <span className="site-toggle">
                <span className="site-toggle-label">Auto Accept Proposals</span>
                <input
                  type="checkbox"
                  checked={site.auto_accept_fast_forward_merge}
                  disabled={updatingSite === site.site}
                  onChange={(event) => onToggleAllowFfm(site.site, event.target.checked)}
                />
              </span>
            </label>
          ))
        )}
      </div>

      <div className="settings-add-site">
        <input
          className="site-input"
          type="text"
          value={newSite}
          onChange={(event) => onNewSiteChange(event.target.value)}
          placeholder="site name"
        />
        <button
          className="icon-button secondary"
          type="button"
          onClick={onAddSite}
          disabled={adding || !newSite.trim()}
        >
          <span>{adding ? "Adding" : "Follow Site"}</span>
        </button>
      </div>
    </section>
  );
}

export default function App() {
  const [rows, setRows] = useState<ProposalRow[]>([]);
  const [siteName, setSiteName] = useState("");
  const [federationSites, setFederationSites] = useState<FederationSitePreference[]>([]);
  const [followAll, setFollowAll] = useState(false);
  const [newFollowSite, setNewFollowSite] = useState("");
  const [selectedKey, setSelectedKey] = useState<string>("");
  const [diff, setDiff] = useState("");
  const [diffError, setDiffError] = useState("");
  const [selectedConflictFile, setSelectedConflictFile] = useState("");
  const [conflictSelections, setConflictSelections] = useState<Record<string, ConflictResolutionChoice>>({});
  const [conflictDetail, setConflictDetail] = useState<ProposalConflictFile | null>(null);
  const [conflictError, setConflictError] = useState("");
  const [loadingRows, setLoadingRows] = useState(true);
  const [loadingDiff, setLoadingDiff] = useState(false);
  const [loadingConflict, setLoadingConflict] = useState(false);
  const [addingFollowSite, setAddingFollowSite] = useState(false);
  const [updatingAllowFfmSite, setUpdatingAllowFfmSite] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<
    "accept" | "reject" | "resolve" | "apply-conflicts" | "reconcile" | null
  >(null);
  const [toast, setToast] = useState<Toast | null>(null);

  const selectedRow = rows.find((row) => proposalRowKey(row) === selectedKey);
  const inboundRows = rows.filter((row) => row.direction === "inbound");
  const outboundRows = rows.filter((row) => row.direction === "outbound");

  const loadRows = useCallback(async () => {
    setLoadingRows(true);
    try {
      const nextRows = await listProposalRows();
      setRows(nextRows);
      setSelectedKey((current) => {
        if (current && nextRows.some((row) => proposalRowKey(row) === current)) {
          return current;
        }
        return nextRows[0] ? proposalRowKey(nextRows[0]) : "";
      });
    } catch (error) {
      setToast({
        tone: "danger",
        message: error instanceof Error ? error.message : "Failed to load proposals"
      });
    } finally {
      setLoadingRows(false);
    }
  }, []);

  const loadFederationSites = useCallback(async () => {
    const response = await listFederationSites();
    setFederationSites(response.sites);
    setFollowAll(response.follow_all);
  }, []);

  useEffect(() => {
    void loadRows();
  }, [loadRows]);

  useEffect(() => {
    loadFederationSites().catch((error) => {
      setToast({
        tone: "danger",
        message:
          error instanceof Error ? error.message : "Failed to load federation site settings"
      });
    });
  }, [loadFederationSites]);

  useEffect(() => {
    getSiteInfo()
      .then((siteInfo) => {
        setSiteName(siteInfo.site_name);
      })
      .catch((error) => {
        setToast({
          tone: "danger",
          message: error instanceof Error ? error.message : "Failed to load site information"
        });
      });
  }, []);

  useEffect(() => {
    if (!selectedRow) {
      setDiff("");
      setDiffError("");
      setSelectedConflictFile("");
      setConflictSelections({});
      setConflictDetail(null);
      setConflictError("");
      return;
    }

    let alive = true;
    setLoadingDiff(true);
    setDiffError("");

    getProposalRowDiff(selectedRow)
      .then((nextDiff) => {
        if (alive) setDiff(nextDiff);
      })
      .catch((error) => {
        if (alive) {
          setDiff("");
          setDiffError(error instanceof Error ? error.message : "Failed to load diff");
        }
      })
      .finally(() => {
        if (alive) setLoadingDiff(false);
      });

    return () => {
      alive = false;
    };
  }, [selectedRow]);

  useEffect(() => {
    const files =
      selectedRow?.direction === "inbound" && selectedRow.state.kind === "Conflicted"
        ? selectedRow.state.files ?? []
        : [];
    setSelectedConflictFile((current) =>
      current && files.includes(current) ? current : files[0] ?? ""
    );
    setConflictSelections((current) => {
      const nextEntries = Object.entries(current).filter(([file]) => files.includes(file));
      return Object.fromEntries(nextEntries) as Record<string, ConflictResolutionChoice>;
    });
  }, [selectedRow]);

  useEffect(() => {
    if (
      !selectedRow ||
      selectedRow.direction !== "inbound" ||
      selectedRow.state.kind !== "Conflicted" ||
      !selectedConflictFile
    ) {
      setConflictDetail(null);
      setConflictError("");
      return;
    }

    let alive = true;
    setLoadingConflict(true);
    setConflictError("");

    getProposalConflictFile(selectedRow.proposal, selectedConflictFile)
      .then((nextDetail) => {
        if (alive) setConflictDetail(nextDetail);
      })
      .catch((error) => {
        if (alive) {
          setConflictDetail(null);
          setConflictError(
            error instanceof Error ? error.message : "Failed to load conflict details"
          );
        }
      })
      .finally(() => {
        if (alive) setLoadingConflict(false);
      });

    return () => {
      alive = false;
    };
  }, [selectedConflictFile, selectedRow]);

  function setConflictChoice(file: string, choice: ConflictResolutionChoice) {
    setConflictSelections((current) => ({
      ...current,
      [file]: choice
    }));
  }

  async function runAction(action: "accept" | "reject" | "resolve" | "reconcile") {
    setBusyAction(action);
    try {
      if (action === "reconcile") {
        const message = await reconcileProposals();
        setToast({ tone: "success", message });
      } else if (selectedRow) {
        const response =
          action === "accept"
            ? await acceptProposal(selectedRow.proposal)
            : action === "reject"
              ? await rejectProposal(selectedRow.proposal)
              : await resolveProposal(selectedRow.proposal);
        setToast({
          tone: response.ok ? "success" : "danger",
          message: response.message
        });
      }
      await loadRows();
    } catch (error) {
      setToast({
        tone: "danger",
        message: error instanceof Error ? error.message : `Failed to ${action}`
      });
    } finally {
      setBusyAction(null);
    }
  }

  async function applyConflictSelections() {
    if (!selectedRow || selectedRow.state.kind !== "Conflicted") {
      return;
    }

    setBusyAction("apply-conflicts");
    try {
      const files = selectedRow.state.files ?? [];
      const resolutions = files.map((file) => ({
        file,
        choice: conflictSelections[file]
      }));
      const response = await resolveProposalConflicts(selectedRow.proposal, resolutions);
      setToast({
        tone: response.ok ? "success" : "danger",
        message: response.message
      });
      await loadRows();
    } catch (error) {
      setToast({
        tone: "danger",
        message:
          error instanceof Error ? error.message : "Failed to apply conflict selections"
      });
    } finally {
      setBusyAction(null);
    }
  }

  async function addSiteToFollowing() {
    const site = newFollowSite.trim();
    if (!site) {
      return;
    }

    setAddingFollowSite(true);
    try {
      const response = await addFollowingSite(site);
      setFederationSites(response.sites);
      setFollowAll(response.follow_all);
      setNewFollowSite("");
      setToast({ tone: "success", message: `Now following ${site}` });
    } catch (error) {
      setToast({
        tone: "danger",
        message: error instanceof Error ? error.message : "Failed to add followed site"
      });
    } finally {
      setAddingFollowSite(false);
    }
  }

  async function updateAllowFfm(site: string, enabled: boolean) {
    setUpdatingAllowFfmSite(site);
    try {
      const response = await setAllowFastForwardMerge(site, enabled);
      setFederationSites(response.sites);
      setFollowAll(response.follow_all);
    } catch (error) {
      setToast({
        tone: "danger",
        message: error instanceof Error ? error.message : "Failed to update auto-accept"
      });
    } finally {
      setUpdatingAllowFfmSite(null);
    }
  }

  const canUseSelectedRef =
    !!selectedRow &&
    selectedRow.direction === "inbound" &&
    selectedRow.hasLocalRef &&
    !terminalStates.has(selectedRow.state.kind) &&
    selectedRow.state.kind !== "Error";
  const canAccept = canUseSelectedRef && selectedRow?.state.kind !== "Conflicted";
  const canReject = canUseSelectedRef;
  const canResolve = canUseSelectedRef && selectedRow?.state.kind === "Conflicted";

  return (
    <main className="app-shell">
      <section className="app-header" aria-label="Proposal overview">
        <div className="title-block">
          <span className="eyebrow">Federation inbox</span>
          <h1>{siteName ? `Git Federation: ${siteName}` : "Git Federation"}</h1>
          <p className="header-subtitle">Proposals</p>
        </div>
        <div className="header-actions">
          <button
            className="icon-button secondary"
            type="button"
            onClick={() => void loadRows()}
            disabled={loadingRows || busyAction !== null}
          >
            <IconRefresh />
            <span>Refresh</span>
          </button>
          <button
            className="icon-button primary"
            type="button"
            onClick={() => void runAction("reconcile")}
            disabled={busyAction !== null}
          >
            <IconBranch />
            <span>{busyAction === "reconcile" ? "Reconciling" : "Reconcile"}</span>
          </button>
        </div>
      </section>

      {toast ? (
        <div className={`toast toast-${toast.tone}`} role="status">
          {toast.message}
        </div>
      ) : null}

      <section className="workspace" aria-label="Proposal workspace">
        <aside className="proposal-list" aria-label="Proposal list">
          <FederationSettings
            sites={federationSites}
            followAll={followAll}
            newSite={newFollowSite}
            onNewSiteChange={setNewFollowSite}
            onAddSite={() => void addSiteToFollowing()}
            onToggleAllowFfm={(site, enabled) => void updateAllowFfm(site, enabled)}
            adding={addingFollowSite}
            updatingSite={updatingAllowFfmSite}
          />
          <div className="list-header">
            <span>{rows.length} total</span>
          </div>
          <div className="list-scroll">
            {loadingRows ? <div className="list-empty">Loading proposals</div> : null}
            {!loadingRows && rows.length === 0 ? (
              <div className="list-empty">No proposals found</div>
            ) : null}
            {rows.length > 0 ? (
              <>
                <div className="list-section-header">
                  <span>Inbound</span>
                  <span>{inboundRows.length}</span>
                </div>
                {inboundRows.map((row) => (
                  <ProposalListItem
                    key={proposalRowKey(row)}
                    row={row}
                    selected={proposalRowKey(row) === selectedKey}
                    onSelect={(row) => setSelectedKey(proposalRowKey(row))}
                  />
                ))}
                <div className="list-section-header">
                  <span>Created here</span>
                  <span>{outboundRows.length}</span>
                </div>
                {outboundRows.map((row) => (
                  <ProposalListItem
                    key={proposalRowKey(row)}
                    row={row}
                    selected={proposalRowKey(row) === selectedKey}
                    onSelect={(row) => setSelectedKey(proposalRowKey(row))}
                  />
                ))}
              </>
            ) : null}
          </div>
        </aside>

        <section className="review-panel" aria-label="Proposal review">
          {selectedRow ? (
            <>
              <div className="review-header">
                <div className="selected-title">
                  <span className="eyebrow">Selected proposal</span>
                  <h2>{proposalLabel(selectedRow.proposal)}</h2>
                </div>
                {selectedRow.direction === "inbound" ? (
                  <div className="review-actions">
                    <button
                      className="icon-button accept"
                      type="button"
                      onClick={() => void runAction("accept")}
                      disabled={!canAccept || busyAction !== null}
                    >
                      <IconCheck />
                      <span>{busyAction === "accept" ? "Accepting" : "Accept"}</span>
                    </button>
                    <button
                      className="icon-button reject"
                      type="button"
                      onClick={() => void runAction("reject")}
                      disabled={!canReject || busyAction !== null}
                    >
                      <IconX />
                      <span>{busyAction === "reject" ? "Rejecting" : "Reject"}</span>
                    </button>
                    {selectedRow.state.kind === "Conflicted" ? (
                    <button
                      className="icon-button secondary"
                      type="button"
                      onClick={() => void runAction("resolve")}
                      disabled={!canResolve || busyAction !== null}
                    >
                      <IconResolve />
                      <span>{busyAction === "resolve" ? "Resolving" : "Resolve"}</span>
                    </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
              <SelectedDetails row={selectedRow} />
              {selectedRow.direction === "inbound" &&
              selectedRow.state.kind === "Conflicted" &&
              selectedRow.state.files?.length ? (
                <ConflictInspector
                  files={selectedRow.state.files}
                  selectedFile={selectedConflictFile}
                  onSelectFile={setSelectedConflictFile}
                  selections={conflictSelections}
                  onSelectChoice={setConflictChoice}
                  onApplySelections={() => void applyConflictSelections()}
                  detail={conflictDetail}
                  loading={loadingConflict}
                  error={conflictError}
                />
              ) : null}
              <DiffView diff={diff} loading={loadingDiff} error={diffError} />
            </>
          ) : (
            <div className="review-empty">No proposal selected</div>
          )}
        </section>
      </section>
    </main>
  );
}
