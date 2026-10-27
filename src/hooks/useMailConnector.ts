import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import {
  isMacPlatform,
  type ConnectorConfigView,
  type ConnectorStatus,
  type MailAccount,
} from "../lib/mail-connector";

// Every query here is macOS-only: off macOS nothing is enabled, so no
// `mail_connector_*` command is ever invoked. None of these log results (they
// carry email addresses) and none ever touch the connector key.

export const MAIL_STATUS_KEY = ["mail-connector", "status"] as const;
export const MAIL_CONFIG_KEY = ["mail-connector", "config"] as const;
export const MAIL_ACCOUNTS_KEY = ["mail-connector", "accounts"] as const;

// `mail_connector_status` reads the keychain, so don't poll it faster than this.
const FAST_POLL_MS = 5_000;
const SLOW_POLL_MS = 30_000;

// How many components currently want the fast rate (the Settings section).
let fastObservers = 0;

/** Polls the connector status: every 5 s while Settings is open (see
 *  `useFastStatusPolling`), every 30 s otherwise. Used by the title-bar dot
 *  and the Settings section, which share one cached query. */
export function useMailConnectorStatus() {
  return useQuery({
    queryKey: MAIL_STATUS_KEY,
    queryFn: () => invoke<ConnectorStatus>("mail_connector_status"),
    enabled: isMacPlatform(),
    refetchInterval: () => (fastObservers > 0 ? FAST_POLL_MS : SLOW_POLL_MS),
    staleTime: 2_000,
    retry: false,
  });
}

/** While mounted (Settings open), switch status polling to the fast rate. */
export function useFastStatusPolling() {
  const qc = useQueryClient();
  useEffect(() => {
    if (!isMacPlatform()) return;
    fastObservers += 1;
    // Refetching now re-evaluates the interval, so the fast rate applies at once.
    void qc.refetchQueries({ queryKey: MAIL_STATUS_KEY });
    return () => {
      fastObservers -= 1;
    };
  }, [qc]);
}

export function useMailConnectorConfig() {
  return useQuery({
    queryKey: MAIL_CONFIG_KEY,
    queryFn: () => invoke<ConnectorConfigView>("mail_connector_get_config"),
    enabled: isMacPlatform(),
    staleTime: 0,
    retry: false,
  });
}

/** Reads Apple Mail's accounts (via osascript, so it can raise the Automation
 *  prompt). Re-read whenever the Settings section mounts, or on Refresh. */
export function useMailAccounts() {
  return useQuery({
    queryKey: MAIL_ACCOUNTS_KEY,
    queryFn: () => invoke<MailAccount[]>("mail_connector_accounts"),
    enabled: isMacPlatform(),
    staleTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}
