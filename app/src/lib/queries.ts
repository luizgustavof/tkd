// Consultas e mutacoes (TanStack Query em cima dos comandos Tauri).
import { QueryClient, useMutation, useQuery, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { commands, type UsageSummary } from "./api";
import { onLiveEvent } from "./live";
import { toast } from "../components/toast";
import { errMsg } from "./api";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { staleTime: 15_000, refetchOnWindowFocus: false, retry: 1 },
    mutations: { onError: (e) => toast(errMsg(e), "error") },
  },
});

export const keys = {
  status: ["status"] as const,
  setup: ["setup"] as const,
  channels: ["channels"] as const,
  channel: (slug: string) => ["channel", slug] as const,
  avatar: (slug: string) => ["avatar", slug] as const,
  agents: ["agents"] as const,
  usage: (days: number) => ["usage", days] as const,
  browser: ["browser"] as const,
};

// etapa terminou / video comecou: atualiza a lista e o canal (agrupado em 400 ms)
let pending = new Set<string>();
let avatars = new Set<string>();
let timer: ReturnType<typeof setTimeout> | null = null;
// sub-etapas (cada chamada do Claude, cada script do navegador) nao mudam nada que a lista/canal mostrem
const SUB_STEPS = new Set(["agent", "browser"]);
// so login e publicacao trocam a foto da conta (antes toda etapa relia todas as fotos em base64)
const AVATAR_STEPS = new Set(["login", "publish"]);
onLiveEvent((e) => {
  if (e.status === "progress" || e.status === "info") return;
  if (e.step === "browser") void queryClient.invalidateQueries({ queryKey: keys.browser });
  if (SUB_STEPS.has(e.step)) return;
  pending.add(e.channel);
  if (AVATAR_STEPS.has(e.step) && e.status !== "start") avatars.add(e.channel);
  if (timer) return;
  timer = setTimeout(() => {
    const chans = pending;
    const avs = avatars;
    pending = new Set();
    avatars = new Set();
    timer = null;
    void queryClient.invalidateQueries({ queryKey: keys.channels });
    void queryClient.invalidateQueries({ queryKey: keys.status });
    for (const c of chans) void queryClient.invalidateQueries({ queryKey: keys.channel(c) });
    for (const c of avs) void queryClient.invalidateQueries({ queryKey: keys.avatar(c) });
  }, 400);
});

export function useAppStatus() {
  return useQuery({ queryKey: keys.status, queryFn: () => commands.appStatus(), staleTime: 5_000 });
}

export function useChannels() {
  return useSuspenseQuery({ queryKey: keys.channels, queryFn: () => commands.listChannels(), refetchInterval: 60_000 });
}

export function useChannelsLoose() {
  return useQuery({ queryKey: keys.channels, queryFn: () => commands.listChannels(), refetchInterval: 60_000 });
}

export function useChannel(slug: string) {
  return useSuspenseQuery({ queryKey: keys.channel(slug), queryFn: () => commands.getChannel(slug) });
}

export function useAvatar(slug: string, enabled: boolean) {
  return useQuery({
    queryKey: keys.avatar(slug),
    queryFn: () => commands.avatar(slug, "tiktok"),
    enabled,
    staleTime: Infinity,
  });
}

export function useAgentConfig() {
  return useSuspenseQuery({ queryKey: keys.agents, queryFn: () => commands.getAgentConfig() });
}

export function useUsage(days: number) {
  return useSuspenseQuery({ queryKey: keys.usage(days), queryFn: () => commands.usageSummary(days) as Promise<UsageSummary>, staleTime: 60_000 });
}

/** Conferencia de primeira execucao (roda comandos externos: cache longo, refaz no botao). */
export function useSetup() {
  return useQuery({ queryKey: keys.setup, queryFn: () => commands.setupCheck(), staleTime: 10 * 60_000 });
}

export function useBrowserStatus() {
  return useQuery({ queryKey: keys.browser, queryFn: () => commands.browserStatus(), refetchInterval: 5_000, staleTime: 0 }); // eventos "browser" ja invalidam na hora
}

/** Mutacao que invalida o canal (e a lista) quando termina. */
export function useChannelMutation<V>(slug: string, fn: (v: V) => Promise<unknown>, ok?: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: fn,
    onSuccess: () => {
      if (ok) toast(ok);
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: keys.channel(slug) });
      void qc.invalidateQueries({ queryKey: keys.channels });
    },
  });
}

/** Cortes gravados pelo clipper (todas as lives) + quem esta sendo vigiado agora. */
export function useClips() {
  return useQuery({ queryKey: ["clips"], queryFn: () => commands.listClips(), refetchInterval: 10_000, staleTime: 0 });
}
