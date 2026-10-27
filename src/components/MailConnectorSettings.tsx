import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useSettings } from "../hooks/useSettings";
import {
  MAIL_CONFIG_KEY,
  MAIL_STATUS_KEY,
  useFastStatusPolling,
  useMailAccounts,
  useMailConnectorConfig,
  useMailConnectorStatus,
} from "../hooks/useMailConnector";
import {
  DEFAULT_FLOW_URL,
  connectIds,
  formatAccountLine,
  isDirectAccount,
  isMacPlatform,
  overallHelpText,
  scriptErrorText,
  type MailAccount,
} from "../lib/mail-connector";

const inputStyle = {
  backgroundColor: "var(--flow-bg-tertiary)",
  color: "var(--flow-text-primary)",
};

const secondaryButtonStyle = {
  backgroundColor: "var(--flow-bg-tertiary)",
  color: "var(--flow-text-primary)",
  border: "1px solid var(--flow-border)",
};

const ERROR_COLOR = "#ef4444";
const OK_COLOR = "#22c55e";

/** Errors from Tauri commands arrive as plain strings. */
function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong";
}

export default function MailConnectorSettings() {
  return (
    <div
      className="flex flex-col gap-3 pt-4 min-w-0"
      style={{ borderTop: "1px solid var(--flow-border)" }}
    >
      <span className="text-sm font-semibold" style={{ color: "var(--flow-text-primary)" }}>
        Mail connector
      </span>
      {isMacPlatform() ? (
        <MailConnectorPanel />
      ) : (
        <p className="text-xs" style={{ color: "var(--flow-text-muted)" }}>
          Mail connector is available on macOS
        </p>
      )}
    </div>
  );
}

function MailConnectorPanel() {
  const qc = useQueryClient();
  useFastStatusPolling();

  const tasksApiUrl = useSettings((s) => s.settings.apiUrl);
  const config = useMailConnectorConfig();
  const accounts = useMailAccounts();
  const status = useMailConnectorStatus();

  // FLOW URL: untouched (null) shows the saved value, else the tasks API URL,
  // else the production default.
  const [urlEdit, setUrlEdit] = useState<string | null>(null);
  const [urlError, setUrlError] = useState<string | null>(null);
  const [urlBusy, setUrlBusy] = useState(false);
  const savedUrl = config.data?.flowUrl ?? null;
  const urlValue =
    urlEdit ?? savedUrl ?? (tasksApiUrl.replace(/\/+$/, "") || DEFAULT_FLOW_URL);
  const urlUnsaved = urlValue.trim() !== (savedUrl ?? "");

  // Key: the field only ever holds what's being typed; the key is never read back.
  const [keyInput, setKeyInput] = useState("");
  const [keyError, setKeyError] = useState<string | null>(null);
  const [keyBusy, setKeyBusy] = useState(false);
  const hasKey = config.data?.hasKey ?? false;

  // Ticks start from the saved selection; nothing is ticked by default.
  const [tickEdit, setTickEdit] = useState<Set<string> | null>(null);
  const [connectError, setConnectError] = useState<string | null>(null);
  const [connectBusy, setConnectBusy] = useState(false);

  const accountList: MailAccount[] = accounts.data ?? [];
  const directIds = new Set(accountList.filter(isDirectAccount).map((a) => a.id));
  const ticked = new Set(
    [...(tickEdit ?? new Set((config.data?.selected ?? []).map((a) => a.id)))].filter(
      (id) => !directIds.has(id),
    ),
  );

  // What Connect sends, and what enables it: one source, so the two agree.
  const idsToConnect = connectIds(accountList, ticked);

  const refreshStatus = () => qc.invalidateQueries({ queryKey: MAIL_STATUS_KEY });
  const refreshConfig = () => qc.invalidateQueries({ queryKey: MAIL_CONFIG_KEY });

  const saveUrl = async (): Promise<boolean> => {
    setUrlError(null);
    try {
      await invoke("mail_connector_set_flow_url", { url: urlValue.trim() });
      setUrlEdit(null);
      await refreshConfig();
      return true;
    } catch (e) {
      setUrlError(errText(e));
      return false;
    }
  };

  const handleSaveUrl = async () => {
    setUrlBusy(true);
    await saveUrl();
    setUrlBusy(false);
  };

  const handleSaveKey = async () => {
    setKeyBusy(true);
    setKeyError(null);
    try {
      await invoke("mail_connector_set_key", { key: keyInput });
      setKeyInput("");
      await Promise.all([refreshConfig(), refreshStatus()]);
    } catch (e) {
      setKeyError(errText(e));
    } finally {
      setKeyBusy(false);
    }
  };

  const handleRemoveKey = async () => {
    setKeyBusy(true);
    setKeyError(null);
    try {
      await invoke("mail_connector_clear_key");
      await Promise.all([refreshConfig(), refreshStatus()]);
    } catch (e) {
      setKeyError(errText(e));
    } finally {
      setKeyBusy(false);
    }
  };

  const toggle = (id: string) => {
    const next = new Set(ticked);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setTickEdit(next);
  };

  const handleConnect = async () => {
    setConnectBusy(true);
    setConnectError(null);
    try {
      if (urlUnsaved && !(await saveUrl())) return;
      if (idsToConnect.length === 0) return;
      await invoke("mail_connector_set_selected", { ids: idsToConnect });
      await invoke("mail_connector_run_now");
      await Promise.all([refreshConfig(), refreshStatus()]);
      // Only once the saved selection is back: dropping the edit earlier
      // would briefly show the old ticks.
      setTickEdit(null);
    } catch (e) {
      setConnectError(errText(e));
    } finally {
      setConnectBusy(false);
    }
  };

  const handleRunNow = async () => {
    setConnectBusy(true);
    setConnectError(null);
    try {
      await invoke("mail_connector_run_now");
      await refreshStatus();
    } catch (e) {
      setConnectError(errText(e));
    } finally {
      setConnectBusy(false);
    }
  };

  const s = status.data;
  const configured = s !== undefined && s.overall !== "not-configured";
  const helpText = overallHelpText(s?.overall);
  const canConnect =
    hasKey && idsToConnect.length > 0 && accounts.data !== undefined && !connectBusy && !urlBusy;
  const now = Date.now();

  return (
    <>
      {/* FLOW URL */}
      <div className="flex flex-col gap-1 min-w-0">
        <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
          FLOW URL
        </label>
        <div className="flex gap-1.5 min-w-0">
          <input
            type="text"
            value={urlValue}
            onChange={(e) => setUrlEdit(e.target.value)}
            placeholder={DEFAULT_FLOW_URL}
            spellCheck={false}
            className="flex-1 min-w-0 text-sm rounded px-2 py-1.5 border-none outline-none"
            style={inputStyle}
          />
          <button
            onClick={handleSaveUrl}
            disabled={!urlUnsaved || !urlValue.trim() || urlBusy}
            className="shrink-0 text-xs font-medium px-2.5 rounded transition-colors disabled:opacity-50"
            style={secondaryButtonStyle}
          >
            Save
          </button>
        </div>
        {urlError && (
          <span className="text-xs break-words" style={{ color: ERROR_COLOR }}>
            {urlError}
          </span>
        )}
      </div>

      {/* Key */}
      <div className="flex flex-col gap-1 min-w-0">
        <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
          Mail connector key
        </label>
        <div className="flex gap-1.5 min-w-0">
          <input
            type="password"
            value={keyInput}
            onChange={(e) => setKeyInput(e.target.value)}
            placeholder="ftsk_..."
            autoComplete="off"
            spellCheck={false}
            className="flex-1 min-w-0 text-sm rounded px-2 py-1.5 border-none outline-none"
            style={inputStyle}
          />
          <button
            onClick={handleSaveKey}
            disabled={!keyInput.trim() || keyBusy}
            className="shrink-0 text-xs font-medium px-2.5 rounded transition-colors disabled:opacity-50"
            style={secondaryButtonStyle}
          >
            Save
          </button>
          {hasKey && (
            <button
              onClick={handleRemoveKey}
              disabled={keyBusy}
              className="shrink-0 text-xs font-medium px-2.5 rounded transition-colors disabled:opacity-50"
              style={secondaryButtonStyle}
            >
              Remove
            </button>
          )}
        </div>
        {hasKey && (
          <span className="text-xs font-medium" style={{ color: OK_COLOR }}>
            Key saved
          </span>
        )}
        {keyError && (
          <span className="text-xs break-words" style={{ color: ERROR_COLOR }}>
            {keyError}
          </span>
        )}
        <span className="text-xs" style={{ color: "var(--flow-text-muted)" }}>
          Create a Mail connector key in FLOW › Settings › API Keys.
        </span>
      </div>

      {/* Accounts */}
      <div className="flex flex-col gap-1.5 min-w-0">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
            Apple Mail accounts
          </span>
          <button
            onClick={() => accounts.refetch()}
            disabled={accounts.isFetching}
            className="text-xs disabled:opacity-50"
            style={{ color: "var(--flow-accent)" }}
          >
            {accounts.isFetching ? "Reading…" : "Refresh"}
          </button>
        </div>
        {accounts.isError && (
          <span className="text-xs break-words" style={{ color: ERROR_COLOR }}>
            {scriptErrorText(accounts.error)}
          </span>
        )}
        {accounts.data?.length === 0 && (
          <span className="text-xs" style={{ color: "var(--flow-text-muted)" }}>
            No accounts found in Apple Mail.
          </span>
        )}
        {accountList.map((a) => (
          <AccountRow
            key={a.id}
            account={a}
            checked={ticked.has(a.id)}
            direct={directIds.has(a.id)}
            onToggle={() => toggle(a.id)}
          />
        ))}
      </div>

      {/* Actions */}
      <div className="flex gap-1.5">
        <button
          onClick={handleConnect}
          disabled={!canConnect}
          className="flex-1 text-xs font-medium py-1.5 rounded transition-colors disabled:opacity-50"
          style={{ backgroundColor: "var(--flow-accent)", color: "#ffffff" }}
        >
          Connect
        </button>
        <button
          onClick={handleRunNow}
          disabled={!configured || connectBusy}
          className="flex-1 text-xs font-medium py-1.5 rounded transition-colors disabled:opacity-50"
          style={secondaryButtonStyle}
        >
          Run now
        </button>
      </div>
      {connectError && (
        <span className="text-xs break-words" style={{ color: ERROR_COLOR }}>
          {connectError}
        </span>
      )}

      {/* Status */}
      {s && (s.running || s.accounts.length > 0 || helpText) && (
        <div className="flex flex-col gap-1 min-w-0">
          {s.running && (
            <span className="text-xs font-medium" style={{ color: "var(--flow-accent)" }}>
              Reading…
            </span>
          )}
          {s.accounts.map((a) => (
            <span
              key={a.id}
              className="text-xs break-words"
              style={{ color: "var(--flow-text-secondary)" }}
            >
              {formatAccountLine(a, now)}
            </span>
          ))}
          {helpText && (
            <span className="text-xs break-words" style={{ color: ERROR_COLOR }}>
              {helpText}
            </span>
          )}
        </div>
      )}
    </>
  );
}

function AccountRow({
  account,
  checked,
  direct,
  onToggle,
}: {
  account: MailAccount;
  checked: boolean;
  direct: boolean;
  onToggle: () => void;
}) {
  return (
    <label
      className={`flex items-start gap-2 min-w-0 ${direct ? "cursor-not-allowed" : "cursor-pointer"}`}
      style={{ opacity: direct ? 0.6 : 1 }}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={direct}
        onChange={onToggle}
        className="rounded mt-0.5 shrink-0"
      />
      <span className="flex flex-col min-w-0 flex-1">
        <span className="flex items-center gap-1.5 min-w-0">
          <span
            className="text-sm truncate"
            style={{ color: "var(--flow-text-primary)" }}
          >
            {account.name}
          </span>
          {account.kind === "iCloud" && (
            <span
              className="shrink-0 text-[10px] leading-none px-1.5 py-0.5 rounded-full"
              style={{
                backgroundColor: "var(--flow-bg-tertiary)",
                color: "var(--flow-text-secondary)",
              }}
            >
              Personal
            </span>
          )}
        </span>
        {account.addresses[0] && (
          <span className="text-xs truncate" style={{ color: "var(--flow-text-muted)" }}>
            {account.addresses[0]}
          </span>
        )}
        {direct && (
          <span className="text-xs" style={{ color: "var(--flow-text-muted)" }}>
            Already connected directly to FLOW
          </span>
        )}
      </span>
    </label>
  );
}
