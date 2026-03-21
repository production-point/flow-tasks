import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettings } from "./useSettings";

export function useLabels() {
  const isConfigured = useSettings((s) => Boolean(s.settings.apiUrl && s.apiKey));
  return useQuery({
    queryKey: ["labels"],
    queryFn: api.getLabels,
    staleTime: 5 * 60 * 1000,
    enabled: isConfigured,
  });
}
