import { useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useClips } from "../lib/queries";
import { commands, type ClipDto } from "../lib/api";
import { ago } from "../lib/format";
import { Dialog, Page, PageHeader, Panel, Segmented } from "../components/ui";
import { toast } from "../components/toast";
import { Select } from "../components/Select";
import { t, tx } from "../lib/i18n";

type Filtro = "todos" | "livres" | "usados";

const PLAT: Record<string, string> = { twitch: "Twitch", kick: "Kick", youtube: "YouTube" };

function nomeLive(u: string) {
  return u.replace(/^https?:\/\/(www\.)?/, "").replace(/\/$/, "");
}

/** emote da Kick/Twitch vira quadradinho: so mostra linha de chat com letra ou numero */
const temTexto = (m: string) => /[\p{L}\p{N}]/u.test(m);

/** Previa: carrega so quando aparece na tela; passar o mouse toca sem som; clicar abre o player. */
function Preview({ src, duration, onOpen }: { src: string; duration: number; onOpen: () => void }) {
  const box = useRef<HTMLButtonElement>(null);
  const vid = useRef<HTMLVideoElement>(null);
  const [visivel, setVisivel] = useState(false);
  const [erro, setErro] = useState(false);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => e?.isIntersecting && (setVisivel(true), io.disconnect()), { rootMargin: "200px" });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return (
    <button
      ref={box}
      type="button"
      aria-label={t("Assistir corte")}
      onClick={onOpen}
      onMouseEnter={() => vid.current?.play().catch(() => {})}
      onMouseLeave={() => {
        const v = vid.current;
        if (v) {
          v.pause();
          v.currentTime = 1;
        }
      }}
      className="group relative block aspect-video w-full overflow-hidden rounded-lg border-0 bg-black p-0"
    >
      {visivel && !erro && (
        <video
          ref={vid}
          src={`${src}#t=1`}
          muted
          loop
          playsInline
          preload="metadata"
          onError={() => setErro(true)}
          className="size-full object-cover"
        />
      )}
      {erro && <span className="absolute inset-0 grid place-items-center text-xs text-dim">{t("Sem prévia")}</span>}
      <span className="pointer-events-none absolute inset-0 grid place-items-center bg-black/25 opacity-100 transition-opacity group-hover:opacity-0">
        <span className="grid size-11 place-items-center rounded-full bg-black/60 text-lg text-white">▶</span>
      </span>
      <span className="mono pointer-events-none absolute end-2 bottom-2 rounded bg-black/70 px-1.5 py-0.5 text-[11px] text-white">
        {duration.toFixed(0)} s
      </span>
    </button>
  );
}

/** Player do app: com som e controles; "Expandir" vai pra tela cheia. */
function Player({ c, src, onClose }: { c: ClipDto; src: string; onClose: () => void }) {
  const vid = useRef<HTMLVideoElement>(null);
  return (
    <div className="flex w-[min(92vw,1100px)] flex-col gap-3 p-4">
      <div className="flex items-baseline gap-2">
        <span className="font-medium">{c.streamer}</span>
        <span className="text-xs text-dim">{PLAT[c.platform] ?? c.platform} · {c.category ?? t("categoria ?")}</span>
        <span className="ms-auto flex gap-2">
          <button type="button" className="btn btn-sec" onClick={() => vid.current?.requestFullscreen().catch(() => {})}>
            {t("Expandir")}
          </button>
          <button type="button" className="btn btn-sec" onClick={onClose}>
            {t("Fechar")}
          </button>
        </span>
      </div>
      <video ref={vid} src={src} controls autoPlay playsInline className="max-h-[75vh] w-full rounded-lg bg-black" />
    </div>
  );
}

function ClipCard({ c }: { c: ClipDto }) {
  const [tocando, setTocando] = useState(false);
  const src = c.mp4 ? convertFileSrc(c.mp4) : "";
  const abrir = useMutation({ mutationFn: () => commands.openClip(c.mp4 ?? "", true) });
  const qc = useQueryClient();
  const [confirmar, setConfirmar] = useState(false);
  useEffect(() => {
    if (!confirmar) return;
    const t = setTimeout(() => setConfirmar(false), 4000);
    return () => clearTimeout(t);
  }, [confirmar]);
  const apagar = useMutation({
    mutationFn: () => commands.deleteClip(c.key),
    onSuccess: () => {
      toast(t("Corte de {s} apagado", { s: c.streamer }));
      return qc.invalidateQueries({ queryKey: ["clips"] });
    },
  });
  return (
    <article className="panel flex min-w-0 flex-col gap-2.5 p-4">
      {src ? (
        <Preview src={src} duration={c.duration ?? 0} onOpen={() => setTocando(true)} />
      ) : (
        <div className="grid aspect-video place-items-center rounded-lg bg-black text-xs text-dim">{t("Arquivo sumiu")}</div>
      )}
      <Dialog open={tocando} onClose={() => setTocando(false)} label={t("Corte de {s}", { s: c.streamer })}>
        <Player c={c} src={src} onClose={() => setTocando(false)} />
      </Dialog>
      <div className="flex items-baseline gap-2">
        <span className="truncate font-medium">{c.streamer}</span>
        <span className="text-xs text-dim">{PLAT[c.platform] ?? c.platform}</span>
        <span className="mono ms-auto text-xs text-dim" title={c.created}>{c.created ? ago(c.created) : ""}</span>
      </div>
      <div className="flex flex-wrap gap-1.5 text-[11px]">
        <span className="rounded-md bg-[#1f2125] px-2 py-0.5 text-fg-2">{c.category ?? t("categoria ?")}</span>
        <span className="mono rounded-md bg-[#1f2125] px-2 py-0.5 text-fg-2">{(c.duration ?? 0).toFixed(0)} s</span>
        <span className="mono rounded-md bg-[#1f2125] px-2 py-0.5 text-fg-2" title={t("nota do momento (chat + audio + movimento)")}>
          {t("nota {n}", { n: (c.score ?? 0).toFixed(2) })}
        </span>
        {c.vertical && <span className="rounded-md bg-accent-soft px-2 py-0.5 text-accent-ink">{t("9:16 pronto")}</span>}
        {c.used ? (
          <span className="rounded-md bg-accent-soft px-2 py-0.5 text-accent-ink">{tx(c.used)}</span>
        ) : (
          <span className="rounded-md bg-[#1f2125] px-2 py-0.5 text-dim">{t("na fila")}</span>
        )}
      </div>
      {c.chat.some(temTexto) && (
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0 text-xs text-muted">
          {c.chat.filter(temTexto).map((m, i) => (
            <li key={i} className="truncate">💬 {m}</li>
          ))}
        </ul>
      )}
      {c.transcript && <p className="m-0 line-clamp-2 text-xs text-dim">“{c.transcript}”</p>}
      <div className="mt-auto flex gap-2 pt-1">
        <button type="button" className="btn btn-sec" disabled={!c.mp4} onClick={() => abrir.mutate()}>
          {t("Mostrar na pasta")}
        </button>
        <button
          type="button"
          className={`btn ms-auto ${confirmar ? "btn-pri bg-live" : "btn-sec"}`}
          disabled={apagar.isPending}
          title={t("Apaga o corte do disco (o post já publicado não é afetado)")}
          onClick={() => (confirmar ? apagar.mutate() : setConfirmar(true))}
        >
          {apagar.isPending ? t("Apagando…") : confirmar ? t("Confirmar?") : t("Deletar")}
        </button>
      </div>
    </article>
  );
}

export default function Clips() {
  const { data, isLoading } = useClips();
  const [filtro, setFiltro] = useState<Filtro>("todos");
  const [streamer, setStreamer] = useState("");
  const clips = data?.clips ?? [];
  const streamers = useMemo(() => [...new Set(clips.map((c) => c.streamer))].sort(), [clips]);
  const shown = clips.filter(
    (c) => (!streamer || c.streamer === streamer) && (filtro === "todos" || (filtro === "usados" ? !!c.used : !c.used)),
  );
  return (
    <Page>
      <PageHeader title={t("Cortes")} sub={t("Tudo que o clipper cortou das lives, mais novo primeiro. Atualiza sozinho a cada 10 s.")} />
      <Panel
        id="vig"
        title={t("Vigiando agora: {n} de {max}", { n: data?.watching.length ?? 0, max: data?.max_watch ?? "?" })}
        sub={data?.watching.length ? data.watching.map(nomeLive).join(" · ") : t("Nenhuma live sendo gravada (piloto desligado, canal pausado ou ninguém ao vivo no jogo).")}
      />
      <div className="flex flex-wrap items-center gap-3">
        <Segmented<Filtro>
          label={t("Filtro")}
          value={filtro}
          onChange={setFiltro}
          options={[
            { value: "todos", label: t("Todos ({n})", { n: clips.length }) },
            { value: "livres", label: t("Na fila") },
            { value: "usados", label: t("Já usados") },
          ]}
        />
        <Select
          label={t("Streamer")}
          size="sm"
          className="w-[200px]"
          value={streamer}
          onChange={setStreamer}
          options={[{ value: "", label: t("Todos os streamers") }, ...streamers.map((s) => ({ value: s, label: s }))]}
        />
      </div>
      {isLoading ? (
        <p className="text-dim">{t("Carregando…")}</p>
      ) : shown.length === 0 ? (
        <p className="text-dim">{t("Nenhum corte ainda.")}</p>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-4">
          {shown.map((c) => (
            <ClipCard key={c.key} c={c} />
          ))}
        </div>
      )}
    </Page>
  );
}
