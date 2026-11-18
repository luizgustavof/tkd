import { useActionState, useEffect, useOptimistic, useState, useTransition } from "react";
import { useFormStatus } from "react-dom";
import { Link, getRouteApi, useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { commands, errMsg, type ChannelDetail, type NoteDto, type Stage } from "../lib/api";
import type { HandleRefs } from "../bindings";
import { Select } from "../components/Select";
import { keys, useChannel } from "../lib/queries";
import { activeStep, forgetLive, useLive } from "../lib/live";
import { effortLabel, hm, modelLabel, shortDate } from "../lib/format";
import { t, tx } from "../lib/i18n";
import type { MsgKey } from "../lib/messages";
import { Avatar, Dialog, Kv, Panel, Strip } from "../components/ui";
import { IBack, INote, IOnce, IPause, IPlay, IShield, ISpin, IX } from "../components/icons";
import { PlatformLogin } from "../components/PlatformLogin";
import { SchedulePicker } from "../components/SchedulePicker";
import { toast } from "../components/toast";

const route = getRouteApi("/canal/$slug");

function Submit({ children, className = "" }: { children: string; className?: string }) {
  const { pending } = useFormStatus();
  return (
    <button className={`btn btn-pri ${className}`} type="submit" disabled={pending}>
      {pending && <ISpin size={14} />}
      {children}
    </button>
  );
}

function useRefresh(slug: string) {
  const qc = useQueryClient();
  return () =>
    Promise.all([qc.invalidateQueries({ queryKey: keys.channel(slug) }), qc.invalidateQueries({ queryKey: keys.channels })]);
}

// ---------------------------------------------------------------- cabecalho

/** Etapas do backend com o estado ao vivo por cima (sem esperar a proxima consulta). */
function useLiveStages(slug: string, stages: Stage[]): Stage[] {
  const a = useLive((s) => s.agents[slug]);
  if (!a || !activeStep(a)) return stages;
  return stages.map((st) => {
    const node = a.steps.findLast((n) => n.key === st.key);
    if (!node) return st;
    return { ...st, state: node.status === "run" ? "now" : node.status === "done" ? "done" : node.status === "error" ? "error" : st.state };
  });
}

function Header({ d }: { d: ChannelDetail }) {
  const s = d.summary;
  const refresh = useRefresh(s.slug);
  const [pausing, startPause] = useTransition();
  const [testing, startTest] = useTransition();
  const [posting, startPost] = useTransition();
  // post de teste publica de verdade (so voce ve): dois cliques, volta sozinho em 4 s
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), 4000);
    return () => clearTimeout(t);
  }, [armed]);
  const stages = useLiveStages(s.slug, s.now.stages);
  const running = useLive((st) => activeStep(st.agents[s.slug]));
  const [editing, setEditing] = useState(false);

  const pause = () =>
    startPause(async () => {
      try {
        await commands.pauseChannel(s.slug, !s.paused);
        toast(s.paused ? t("{s} voltou a postar", { s: s.slug }) : t("{s} pausado", { s: s.slug }));
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await refresh();
    });
  const test = () =>
    startTest(async () => {
      try {
        await commands.runTestVideo(s.slug);
        toast(t("Vídeo de teste começou. Acompanhe em Ao vivo."));
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await refresh();
    });

  const testPost = () => {
    if (!armed) return setArmed(true);
    setArmed(false);
    startPost(async () => {
      try {
        await commands.runTestPost(s.slug);
        toast(t("Post de teste começou: sai como Somente você no TikTok e não listado no YouTube. Acompanhe em Ao vivo."));
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await refresh();
    });
  };

  return (
    <header className="flex flex-col gap-[18px]">
      <Link to="/" className="inline-flex items-center gap-1.5 self-start text-[13px] !text-muted hover:!text-fg">
        <IBack size={14} />
        {t("Canais")}
      </Link>
      <div className="flex flex-wrap items-start gap-x-6 gap-y-4">
        <div className="flex flex-[1_1_460px] items-start gap-[18px]">
          <Avatar slug={s.slug} color={s.color} hasPhoto={s.has_avatar} size={56} ring={s.busy ? "#8b8cff" : "#3a3d44"} />
          <div className="flex min-w-0 flex-col gap-1.5">
            <div className="flex flex-wrap items-baseline gap-2.5">
              <h1 className="m-0 text-[28px] font-semibold tracking-[-0.025em]">{s.slug}</h1>
              {s.tiktok && <span className="mono text-[13px] text-muted"><bdi>@{s.tiktok}</bdi></span>}
              {s.paused && <span className="rounded-[5px] bg-[#1f2125] px-[7px] py-px text-[11px] font-medium text-warn">{t("pausado")}</span>}
            </div>
            <p className="m-0 max-w-[64ch] text-[15px] text-fg-2">{d.persona || s.niche || t("Ainda sem persona. Ela aparece depois do primeiro estudo.")}</p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button className="btn btn-ghost" type="button" onClick={() => setEditing(true)}>
            {t("Editar")}
          </button>
          {!s.is_test && (
            <button className="btn btn-sec" type="button" onClick={pause} disabled={pausing}>
              {s.paused ? <IPlay /> : <IPause />}
              {s.paused ? t("Retomar canal") : t("Pausar canal")}
            </button>
          )}
          <button
            className={`btn ${armed ? "btn-danger" : "btn-sec"}`}
            type="button"
            onClick={testPost}
            disabled={posting || s.busy || !!running}
            title={t("Faz um vídeo e posta de verdade, mas só você vê: Somente você no TikTok, não listado no YouTube. Não conta nos posts do dia.")}
          >
            {posting && <ISpin size={14} />}
            {armed ? t("Confirmar post de teste") : t("Post de teste")}
          </button>
          <button
            className="btn btn-pri"
            type="button"
            onClick={test}
            disabled={testing || s.busy || !!running}
            title={t("Faz um vídeo completo e deixa pra você ver. Não posta.")}
          >
            {(testing || s.busy) && <ISpin size={14} />}
            {s.busy ? t("Fazendo vídeo…") : t("Fazer vídeo de teste")}
          </button>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2.5">
        <Strip stages={stages} labels />
        <span className="text-[13px] text-muted">
          {running ? tx(running.children.findLast((c) => c.status === "run")?.action || running.action || running.label) : t("O teste faz o vídeo inteiro e para antes de postar.")}
        </span>
      </div>
      <Dialog open={editing} onClose={() => setEditing(false)} label={t("Editar {s}", { s: s.slug })}>
        <EditChannel d={d} onClose={() => setEditing(false)} />
      </Dialog>
    </header>
  );
}

// ---------------------------------------------------------------- editar

function EditChannel({ d, onClose }: { d: ChannelDetail; onClose: () => void }) {
  const s = d.summary;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [slug, setSlug] = useState(s.slug);
  const [niche, setNiche] = useState(s.niche);
  const [persona, setPersona] = useState(d.persona);
  const [brand, setBrand] = useState(d.brand);
  const [tags, setTags] = useState(d.hashtags.join(" "));
  const [format, setFormat] = useState(d.format);
  const [chars, setChars] = useState(d.characters.join("\n"));
  const [lang, setLang] = useState(d.edit_lang);
  const [games, setGames] = useState(d.games.join("\n"));
  const [streamers, setStreamers] = useState(d.streamers.join("\n"));
  const [error, setError] = useState<string | null>(null);
  const [pending, start] = useTransition();
  const cleanSlug = slug.trim().toLowerCase();
  const slugOk = /^[a-z0-9_-]{1,40}$/.test(cleanSlug);

  const save = () =>
    start(async () => {
      setError(null);
      try {
        const list = tags.split(/[\s,]+/).map((t) => t.trim()).filter(Boolean);
        const charList = chars.split(/\n|,/).map((c) => c.trim()).filter(Boolean);
        if (format === "edit" && !charList.length) {
          setError(t("Canal de edit precisa de pelo menos um personagem"));
          return;
        }
        const gameList = games.split("\n").map((g) => g.trim()).filter(Boolean);
        const liveList = streamers.split(/\s+/).map((u) => u.trim()).filter(Boolean);
        if (format === "clips" && (!liveList.length || liveList.some((u) => !/^https?:\/\//.test(u)))) {
          setError(t("Cole o link completo de cada live (twitch.tv/…, kick.com/…, youtube.com/@…)"));
          return;
        }
        const changed =
          niche.trim() !== s.niche ||
          persona.trim() !== d.persona ||
          brand.trim() !== d.brand ||
          list.join(" ") !== d.hashtags.join(" ") ||
          format !== d.format ||
          charList.join("|") !== d.characters.join("|") ||
          lang !== d.edit_lang ||
          liveList.join("|") !== d.streamers.join("|") ||
          gameList.join("|") !== d.games.join("|");
        if (changed)
          await commands.updateChannel(s.slug, {
            niche: niche.trim(),
            persona: persona.trim(),
            brand: brand.trim(),
            hashtags: list,
            format,
            characters: charList,
            edit_lang: lang,
            streamers: liveList,
            games: gameList,
          });
        let target = s.slug;
        if (cleanSlug !== s.slug) {
          target = await commands.renameChannel(s.slug, cleanSlug);
          forgetLive(s.slug);
          qc.removeQueries({ queryKey: keys.channel(s.slug) });
        }
        await qc.invalidateQueries({ queryKey: keys.channels });
        await qc.invalidateQueries({ queryKey: keys.channel(target) });
        onClose();
        if (target !== s.slug) await navigate({ to: "/canal/$slug", params: { slug: target } });
        toast(target !== s.slug ? t("Agora o canal se chama {s}", { s: target }) : t("Canal atualizado"));
      } catch (e) {
        setError(errMsg(e));
      }
    });

  const field = "flex flex-col gap-1.5 text-[13px] text-muted";
  return (
    <form
      className="flex w-full flex-col"
      onSubmit={(e) => {
        e.preventDefault();
        if (slugOk) save();
      }}
    >
      <div className="flex flex-col gap-4 px-[22px] pt-5 pb-4">
        <h2 className="m-0 text-[17px] font-semibold">{t("Editar canal")}</h2>
        <label className={field}>
          {t("Nome")}
          <input type="text" value={slug} onChange={(e) => setSlug(e.target.value)} autoComplete="off" className="mono w-full" />
          <span className="text-xs text-faint">
            {slugOk ? t("Letras minúsculas, números, - e _. Começar com _ deixa o canal como teste (nunca posta sozinho).") : t("Use só letras minúsculas, números, - e _.")}
          </span>
        </label>
        <label className={field}>
          {t("Assinatura")}
          <input type="text" value={brand} onChange={(e) => setBrand(e.target.value)} placeholder="MARCA" className="w-full" />
          <span className="text-xs text-faint">{t("Como o canal assina no fim do vídeo e na marca d'água. Nome ou @ dos perfis estudados nunca entra.")}</span>
        </label>
        <label className={field}>
          {t("Tipo de vídeo")}
          <Select
            label={t("Tipo de vídeo")}
            value={format}
            onChange={setFormat}
            options={[
              { value: "texto", label: t("Vídeo de texto"), hint: t("Frases, fatos e cartelas montados pelo agente") },
              { value: "edit", label: t("Edit de personagem"), hint: t("Cenas e falas reais de um personagem por vídeo") },
              { value: "clips", label: t("Cortes de live"), hint: t("Momentos de lives da Twitch, Kick e YouTube, com crédito") },
            ]}
          />
        </label>
        {format === "clips" && (
          <label className={field}>
            {t("Lives")}
            <textarea value={streamers} onChange={(e) => setStreamers(e.target.value)} rows={4} className="mono w-full resize-y" placeholder={"https://twitch.tv/streamer\nhttps://kick.com/streamer"} />
            <span className="text-xs text-faint">
              {t("Um link por linha. Com o piloto ligado, o clipper assiste quem estiver ao vivo (até 2 lives ao mesmo tempo) e o canal posta os melhores cortes com crédito. Use só streamers que liberam corte.")}
            </span>
          </label>
        )}
        {format === "clips" && (
          <label className={field}>
            {t("Jogos")}
            <textarea value={games} onChange={(e) => setGames(e.target.value)} rows={2} className="mono w-full resize-y" placeholder="Grand Theft Auto VI" />
            <span className="text-xs text-faint">
              {t("Um por linha, com o nome da categoria na Twitch/Kick. O clipper só assiste quem estiver nesses jogos e pausa quando o streamer troca de jogo ou passa reprise. Vazio = qualquer jogo.")}
            </span>
          </label>
        )}
        {format === "edit" && (
          <>
            <label className={field}>
              {t("Personagens")}
              <textarea value={chars} onChange={(e) => setChars(e.target.value)} rows={4} className="w-full resize-y" placeholder={"Toji Fushiguro\nLight Yagami"} />
              <span className="text-xs text-faint">{t("Um por linha. Cada vídeo usa um só, em rodízio. O nome completo ajuda a achar as cenas certas.")}</span>
            </label>
            <label className={field}>
              {t("Idioma das falas")}
              <Select
                label={t("Idioma das falas")}
                value={lang}
                onChange={setLang}
                options={[
                  { value: "auto", label: t("Automático") },
                  { value: "ja", label: t("Japonês"), hint: t("Áudio original") },
                  { value: "en", label: t("Inglês"), hint: t("Dublado") },
                  { value: "pt", label: t("Português"), hint: t("Dublado") },
                ]}
              />
            </label>
          </>
        )}
        <label className={field}>
          {t("Nicho")}
          <input type="text" value={niche} onChange={(e) => setNiche(e.target.value)} placeholder={t("ex.: edits de anime, Jujutsu Kaisen")} className="w-full" />
        </label>
        <label className={field}>
          {t("Persona (o texto embaixo do nome)")}
          <textarea value={persona} onChange={(e) => setPersona(e.target.value)} rows={3} className="w-full resize-y" placeholder={t("Como o canal fala e o que ele posta. O estudo preenche sozinho.")} />
        </label>
        <label className={field}>
          {t("Hashtags fixas")}
          <input type="text" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="#anime #edit" className="mono w-full" />
          <span className="text-xs text-faint">{t("Entram em todo post, junto com as que o agente escolhe pro vídeo (até 5 no total).")}</span>
        </label>
        {error && <span role="alert" className="text-[13px] text-live">{error}</span>}
      </div>
      <div className="flex items-center justify-end gap-2 border-t border-line-soft px-[22px] py-4">
        <button className="btn btn-ghost" type="button" onClick={onClose}>
          {t("Cancelar")}
        </button>
        <button className="btn btn-pri" type="submit" disabled={!slugOk || pending}>
          {pending && <ISpin size={14} />}
          {t("Salvar")}
        </button>
      </div>
    </form>
  );
}

// ---------------------------------------------------------------- observacoes

function Notes({ slug, notes }: { slug: string; notes: NoteDto[] }) {
  const refresh = useRefresh(slug);
  const [list, removeOpt] = useOptimistic(notes, (l: NoteDto[], idx: number) => l.filter((n) => n.index !== idx));
  const [, start] = useTransition();
  const [error, action] = useActionState(async (_: string | null, fd: FormData) => {
    const text = String(fd.get("text") ?? "").trim();
    if (!text) return t("Escreva a observação");
    try {
      await commands.addNote(slug, text, fd.get("once") === "on");
      await refresh();
      return null;
    } catch (e) {
      return errMsg(e);
    }
  }, null);
  const del = (n: NoteDto) =>
    start(async () => {
      removeOpt(n.index);
      try {
        await commands.deleteNote(slug, n.index);
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await refresh();
    });

  return (
    <Panel id="obs" title={t("Observações pro agente")} sub={t("Do seu jeito, em português normal. O agente lê isso antes de criar e de avaliar cada vídeo.")}>
      <form action={action} className="flex flex-col gap-2.5 px-5 pb-4">
        <label htmlFor="obs-text" className="sr-only">
          {t("Nova observação")}
        </label>
        <textarea
          id="obs-text"
          name="text"
          rows={3}
          placeholder={t("Ex.: usar mais cenas na chuva; nada de música animada; legenda sempre em minúsculo")}
        />
        <div className="flex flex-wrap items-center gap-2.5">
          <label className="inline-flex items-center gap-2 text-xs text-dim">
            <input type="checkbox" name="once" className="accent-[#8b8cff]" />
            {t("Só no próximo vídeo")}
          </label>
          {error && <span role="alert" className="text-xs text-live">{error}</span>}
          <Submit className="btn-sm ms-auto">{t("Adicionar")}</Submit>
        </div>
      </form>
      {list.map((n) => (
        <div key={n.index} className={`grid grid-cols-[16px_minmax(0,1fr)_auto] items-start gap-2.5 border-t border-line-soft px-5 py-3 ${n.once ? "opacity-75" : ""}`}>
          {n.once ? <IOnce className="mt-0.5 text-dim" /> : <INote className="mt-0.5 text-dim" />}
          <div className="flex flex-col gap-0.5">
            <span>{n.text}</span>
            <span className="text-xs text-dim">{n.once ? t("some depois de usar uma vez") : n.created_at ? t("adicionada {d}", { d: shortDate(n.created_at) }) : t("vale até você apagar")}</span>
          </div>
          <button className="icon-btn" type="button" aria-label={t("Apagar observação")} onClick={() => del(n)}>
            <IX size={14} />
          </button>
        </div>
      ))}
      {!list.length && <p className="m-0 border-t border-line-soft px-5 py-3.5 text-[13px] text-dim">{t("Nenhuma observação. O agente segue só o estilo aprendido.")}</p>}
    </Panel>
  );
}

// ---------------------------------------------------------------- inspiracoes

function studiedLabel(r: HandleRefs | undefined) {
  if (!r || !r.studied) return t("ainda não estudado");
  return r.studied === 1 ? t("{n} vídeo estudado", { n: r.studied }) : t("{n} vídeos estudados", { n: r.studied });
}

function Inspirations({ slug, main, extra, refs, liveClips }: { slug: string; main: string; extra: string[]; refs: HandleRefs[]; liveClips?: boolean }) {
  const refresh = useRefresh(slug);
  const all = [main, ...extra].filter((h) => h && h !== "none");
  const [list, setOpt] = useOptimistic(all);
  const [, start] = useTransition();
  const [error, action] = useActionState(async (_: string | null, fd: FormData) => {
    const h = String(fd.get("handle") ?? "").trim();
    if (!h) return t("Cole o @ ou o link do perfil");
    try {
      await commands.addInspiration(slug, h);
      await refresh();
      toast(t("Adicionado. Clique em Estudar na linha dele quando quiser que o canal aprenda com ele."));
      return null;
    } catch (e) {
      return errMsg(e);
    }
  }, null);
  const busy = useLive((st) => !!activeStep(st.agents[slug]));
  const [studying, startStudy] = useTransition();
  const study = (handle: string | null) =>
    startStudy(async () => {
      try {
        await commands.studyNow(slug, handle);
        toast(
          handle
            ? t("Estudando @{h}: baixa os vídeos novos dele e atualiza o estilo. Acompanhe em Ao vivo.", { h: handle })
            : t("Estudo começou: baixa os vídeos novos de todos e atualiza o estilo. Acompanhe em Ao vivo."),
        );
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await refresh();
    });
  const remove = (h: string) =>
    start(async () => {
      setOpt(list.filter((x) => x !== h));
      try {
        await commands.removeInspiration(slug, h);
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await refresh();
    });

  return (
    <Panel
      id="insp"
      title={slug.startsWith("_") ? t("Canais que o canal estuda") : t("Canais que o {s} estuda", { s: slug })}
      sub={t("Estudar baixa só os vídeos novos do perfil. Se nada mudou desde o último estudo, não gasta tokens.")}
      actions={
        <button className="btn btn-pri btn-sm" type="button" onClick={() => study(null)} disabled={studying || busy || list.length < 2}
          title={t("Busca os vídeos novos de todos os perfis da lista")}>
          {(studying || busy) && <ISpin size={14} />}
          {busy ? t("Trabalhando…") : t("Estudar todos")}
        </button>
      }
    >
      <form action={action} className="flex flex-wrap items-end gap-2 px-5 pb-4">
        <label className="flex flex-[1_1_300px] flex-col gap-1.5 text-[13px] font-medium">
          {t("Adicionar canal")}
          <input type="text" name="handle" placeholder={t("@perfil ou link do TikTok")} className="w-full" autoComplete="off" />
        </label>
        <Submit>{t("Adicionar")}</Submit>
        {error && <span role="alert" className="basis-full text-xs text-live">{error}</span>}
      </form>
      {list.map((h, i) => (
        <div key={h} className="flex flex-wrap items-center gap-x-4 gap-y-3 border-t border-line-soft px-5 py-3.5">
          <div className="flex flex-[1_1_240px] flex-col">
            <span className="mono font-medium"><bdi>@{h}</bdi></span>
            <span className="text-[13px] text-muted">
              {i === 0 ? t("principal") : t("inspiração extra")} · {studiedLabel(refs.find((r) => r.handle === h.toLowerCase()))}
            </span>
          </div>
          <button className="btn btn-sm" type="button" onClick={() => study(h)} disabled={studying || busy}
            title={t("Busca só os vídeos novos de @{h} e atualiza o estilo", { h })}>
            {t("Estudar")}
          </button>
          <a className="btn btn-ghost btn-sm" href={`https://www.tiktok.com/@${h}`} target="_blank" rel="noreferrer">
            {t("Abrir perfil")}
          </a>
          <button className="icon-btn" type="button" aria-label={t("Remover @{h}", { h })} onClick={() => remove(h)}>
            <IX size={14} />
          </button>
        </div>
      ))}
      {!list.length &&
        (liveClips ? (
          <p className="m-0 border-t border-line-soft px-5 py-3.5 text-[13px] text-dim">
            {t("Não precisa em canal de cortes: os cortes saem das lives dos streamers e o Claude escolhe pelo momento, chat e categoria. O estudo não entra nos cortes. Pra mudar o tom de título e legenda, use as observações do canal.")}
          </p>
        ) : (
          <p className="m-0 border-t border-line-soft px-5 py-3.5 text-[13px] text-warn">{t("Sem canal pra estudar. Adicione pelo menos um.")}</p>
        ))}
    </Panel>
  );
}

// ---------------------------------------------------------------- contas

function Accounts({ d }: { d: ChannelDetail }) {
  const s = d.summary;
  return (
    <Panel id="conta" title={t("Conta")} sub={t("Onde {s} posta. Cada conta fica logada numa janela só desse canal.", { s: s.slug })}>
      <PlatformLogin key={`${s.slug}:tiktok`} slug={s.slug} platform="tiktok" handle={s.tiktok} compact />
      <PlatformLogin key={`${s.slug}:youtube`} slug={s.slug} platform="youtube" handle={s.youtube} compact />
      <div className="flex items-start gap-2 border-t border-line-soft px-5 pt-3 pb-4 text-[13px] text-muted">
        <IShield className="mt-0.5 flex-none" />
        <span>{t("Antes de postar, o tkd confere se está na conta certa. Se não estiver, não posta. Cada vídeo aprovado vai pra todas as redes conectadas, público (no YouTube como Short).")}</span>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------- coluna direita

function AgentSummary({ d }: { d: ChannelDetail }) {
  const def = d.picks[0];
  return (
    <Panel id="agente" title={t("Inteligência do agente")} sub={t("Usa o padrão de todos os canais, menos onde você mudar.")}>
      <div className="px-5 pb-3.5">
        {def && (
          <Kv k={t("Padrão")}>
            <span className="text-[13px] text-fg-2">
              {d.agent_model || d.agent_effort
                ? t("{m} · esforço {e}", { m: modelLabel(d.agent_model || def.model), e: effortLabel(d.agent_effort || def.effort) })
                : t("padrão do app ({m})", { m: def.model_label })}
            </span>
          </Kv>
        )}
        {d.picks.map((p) => {
          const own = p.model_from.startsWith("channel") || p.effort_from.startsWith("channel") || p.model_from === "global_task" || p.effort_from === "global_task";
          return (
            <Kv key={p.task} k={tx(p.label)}>
              {own ? (
                <span className="rounded-md bg-accent-soft px-2 py-[3px] text-xs font-medium whitespace-nowrap text-accent-ink">
                  {p.model_label} · {effortLabel(p.effort)}
                </span>
              ) : (
                <span className="text-[13px] text-dim">{t("padrão")}</span>
              )}
            </Kv>
          );
        })}
      </div>
      <div className="px-5 pb-[18px]">
        <Link to="/agentes" className="text-[13px] font-medium">
          {t("Mudar nos agentes")}
        </Link>
      </div>
    </Panel>
  );
}

function Learned({ d }: { d: ChannelDetail }) {
  const l = d.learned;
  return (
    <Panel id="mem" title={d.summary.slug.startsWith("_") ? t("O que o canal já aprendeu") : t("O que o {s} já aprendeu", { s: d.summary.slug })}>
      <div className="px-5 pb-4">
        <Kv k={t("Guia de estilo")}>
          <span className={`text-[13px] ${l.has_style_card ? "text-ok" : "text-warn"}`}>{l.has_style_card ? t("pronto") : t("ainda não")}</span>
        </Kv>
        <Kv k={t("Vídeos de referência estudados")}>
          <span className="mono text-[13px] text-fg-2">
            {l.refs_analyzed} / {l.refs_total}
          </span>
        </Kv>
        <Kv k={t("Vídeos aprovados")}>
          <span className="mono text-[13px] text-fg-2">
            {l.library}
            {l.avg_score != null ? ` · ${t("nota {n}", { n: l.avg_score.toFixed(1) })}` : ""}
          </span>
        </Kv>
        {l.style_excerpt && <p className="m-0 mt-2 line-clamp-4 rounded-lg bg-well p-3 text-[13px] whitespace-pre-line text-muted">{l.style_excerpt}</p>}
        {l.last_problems.length > 0 && (
          <div className="mt-3 flex flex-col gap-1.5">
            <span className="text-xs font-medium text-dim">{t("Últimas críticas do avaliador")}</span>
            {l.last_problems.map((p, i) => (
              <span key={i} className="text-[13px] text-fg-2">
                {p.replace(/^\S+\s/, "")}
              </span>
            ))}
          </div>
        )}
      </div>
    </Panel>
  );
}

const RUN_LABEL: Record<string, [MsgKey, string]> = {
  posted: ["Postado", "text-ok"],
  ok: ["Aprovado (teste)", "text-ok"],
  reprovado: ["Reprovado", "text-warn"],
  erro: ["Erro", "text-live"],
  start: ["Rodando", "text-accent"],
};

function Runs({ d }: { d: ChannelDetail }) {
  if (!d.runs.length) return null;
  return (
    <Panel id="runs" title={t("Últimos vídeos")}>
      <div className="px-5 pb-4">
        {d.runs.map((r) => {
          const [key, cls] = RUN_LABEL[r.status] ?? [null, "text-muted"];
          const label = key ? t(key) : r.status;
          return (
            <Kv key={r.run_id} k={<span className="mono text-[13px] text-muted">{shortDate(r.at)} · {hm(r.at)}</span>}>
              <span className={`max-w-[220px] truncate text-[13px] ${cls}`} title={r.detail}>
                {label}
                {r.status === "reprovado" && r.detail ? ` · ${r.detail}` : ""}
              </span>
            </Kv>
          );
        })}
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------- excluir

function DeleteChannel({ d }: { d: ChannelDetail }) {
  const [open, setOpen] = useState(false);
  const busy = d.summary.busy;
  return (
    <Panel id="excluir" title={t("Excluir canal")} sub={t("Tira o canal do app e desliga o login das contas. Dá pra recuperar pela pasta .lixeira.")}>
      <div className="flex items-center justify-between gap-4 border-t border-line-soft px-5 py-4">
        <span className="text-[13px] text-muted">{busy ? t("O canal está rodando agora. Espere terminar.") : t("O histórico dele some do Ao vivo e do Consumo.")}</span>
        <button className="btn btn-sec btn-sm shrink-0 !text-live" type="button" onClick={() => setOpen(true)} disabled={busy}>
          {t("Excluir canal")}
        </button>
      </div>
      <Dialog open={open} onClose={() => setOpen(false)} label={t("Excluir {s}", { s: d.summary.slug })}>
        <ConfirmDelete slug={d.summary.slug} onClose={() => setOpen(false)} />
      </Dialog>
    </Panel>
  );
}

function ConfirmDelete({ slug, onClose }: { slug: string; onClose: () => void }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [typed, setTyped] = useState("");
  const [pending, start] = useTransition();
  const ok = typed.trim() === slug;
  const del = () =>
    start(async () => {
      try {
        const dst = await commands.deleteChannel(slug);
        onClose();
        forgetLive(slug);
        qc.removeQueries({ queryKey: keys.channel(slug) });
        qc.removeQueries({ queryKey: ["usage"] });
        await qc.invalidateQueries({ queryKey: keys.channels });
        await navigate({ to: "/" });
        toast(t("{s} excluído. Cópia em {d}", { s: slug, d: dst }));
      } catch (e) {
        toast(errMsg(e), "error");
      }
    });
  return (
    <form
      className="flex w-full flex-col"
      onSubmit={(e) => {
        e.preventDefault();
        if (ok) del();
      }}
    >
      <div className="flex flex-col gap-2 px-[22px] pt-5 pb-4">
        <h2 className="m-0 text-[17px] font-semibold">{t("Excluir {s}?", { s: slug })}</h2>
        <p className="m-0 text-[14px] text-fg-2">
          {t("O canal some do app e o piloto deixa de postar nele. As configurações, a biblioteca e os logins do TikTok e do YouTube vão pra pasta .lixeira. O histórico dele no Ao vivo e no Consumo é apagado. Nada muda nas redes.")}
        </p>
        <label className="mt-2 flex flex-col gap-1.5 text-[13px] text-muted">
          <span>
            {t("Digite")} <b className="mono text-fg">{slug}</b> {t("pra confirmar")}
          </span>
          <input type="text" value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" autoFocus className="w-full" />
        </label>
      </div>
      <div className="flex items-center justify-end gap-2 border-t border-line-soft px-[22px] py-4">
        <button className="btn btn-ghost" type="button" onClick={onClose}>
          {t("Cancelar")}
        </button>
        <button className="btn btn-danger" type="submit" disabled={!ok || pending}>
          {pending && <ISpin size={14} />}
          {t("Excluir canal")}
        </button>
      </div>
    </form>
  );
}

export default function Channel() {
  const { slug } = route.useParams();
  const { data: d } = useChannel(slug);
  return (
    <div className="flex max-w-[1120px] flex-col gap-[26px]">
      <Header d={d} />
      <div className="flex flex-wrap items-start gap-6">
        <div className="flex min-w-0 flex-[3_1_520px] flex-col gap-6">
          <Notes slug={slug} notes={d.notes} />
          <Inspirations slug={slug} main={d.ref_handle} extra={d.inspirations} refs={d.learned.refs_by_handle} liveClips={d.format === "clips"} />
          <Accounts d={d} />
        </div>
        <div className="flex min-w-0 flex-[2_1_340px] flex-col gap-6">
          <Panel id="agenda" title={t("Quando postar")} sub={d.summary.is_test ? t("Canal de teste: só roda quando você manda.") : t("Escolha os dias e os horários dos vídeos.")}>
            <SchedulePicker slug={slug} schedule={d.schedule} nextLabel={d.summary.next_post_label} />
          </Panel>
          <AgentSummary d={d} />
          <Learned d={d} />
          <Runs d={d} />
          <DeleteChannel d={d} />
        </div>
      </div>
    </div>
  );
}
