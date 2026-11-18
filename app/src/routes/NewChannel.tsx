import { useActionState, useState } from "react";
import { useFormStatus } from "react-dom";
import { Link, useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { commands, errMsg } from "../lib/api";
import { keys } from "../lib/queries";
import { Panel, Skeleton } from "../components/ui";
import { ICheck, ILock, IShield, ISpin, IX } from "../components/icons";
import { PlatformLogin } from "../components/PlatformLogin";
import { SchedulePicker } from "../components/SchedulePicker";
import { toast } from "../components/toast";
import { t } from "../lib/i18n";
import type { MsgKey } from "../lib/messages";

const STEPS: MsgKey[] = ["Nome e estilo", "Contas", "Canais pra estudar", "Quando postar"];

function Stepper({ at }: { at: number }) {
  return (
    <ol aria-label={t("Etapas")} className="m-0 flex list-none gap-3 overflow-x-auto p-0">
      {STEPS.map((s, i) => {
        const st = i < at ? "done" : i === at ? "now" : "todo";
        return (
          <li
            key={s}
            aria-current={st === "now" ? "step" : undefined}
            className={`flex min-w-0 items-center gap-2.5 text-[13px] whitespace-nowrap ${i < STEPS.length - 1 ? "flex-1" : "flex-none"} ${
              st === "done" ? "text-fg-2" : st === "now" ? "font-medium text-fg" : "text-dim"
            }`}
          >
            <b
              className={`grid size-6 flex-none place-items-center rounded-full text-xs font-semibold ${
                st === "done" ? "bg-ok-bg text-ok" : st === "now" ? "bg-accent text-bg" : "bg-[#1f2125] text-muted"
              }`}
            >
              {st === "done" ? <ICheck size={12} /> : i + 1}
            </b>
            {t(s)}
            {i < STEPS.length - 1 && <span className="h-px min-w-3 flex-auto bg-line" />}
          </li>
        );
      })}
    </ol>
  );
}

function Submit({ children }: { children: string }) {
  const { pending } = useFormStatus();
  return (
    <button className="btn btn-pri" type="submit" disabled={pending}>
      {pending && <ISpin size={14} />}
      {children}
    </button>
  );
}

function StepName({ onDone }: { onDone: (slug: string) => void }) {
  const qc = useQueryClient();
  const [error, action] = useActionState(async (_: string | null, fd: FormData) => {
    const slug = String(fd.get("slug") ?? "").trim().toLowerCase();
    if (!slug) return t("Dê um nome curto pro canal");
    try {
      const s = await commands.createChannel({ slug, ref_handle: String(fd.get("ref") ?? ""), niche: String(fd.get("niche") ?? "") });
      await qc.invalidateQueries({ queryKey: keys.channels });
      onDone(s);
      return null;
    } catch (e) {
      return errMsg(e);
    }
  }, null);
  return (
    <form action={action} className="flex flex-col gap-4 px-5 pb-5">
      <label className="flex flex-col gap-1.5 text-[13px] font-medium">
        {t("Nome do canal")}
        <input type="text" name="slug" placeholder={t("ex.: noir")} autoComplete="off" pattern="[a-z0-9_\-]+" title={t("só letras minúsculas, números, - e _")} />
        <span className="text-xs font-normal text-dim">{t("Só letras minúsculas e números. Começar com _ cria um canal de teste, que nunca posta sozinho.")}</span>
      </label>
      <label className="flex flex-col gap-1.5 text-[13px] font-medium">
        {t("Do que é o canal")}
        <textarea name="niche" rows={2} placeholder={t("ex.: edits sombrios de filmes noir, um personagem por vídeo")} />
      </label>
      <label className="flex flex-col gap-1.5 text-[13px] font-medium">
        {t("Canal principal pra estudar")}
        <input type="text" name="ref" placeholder={t("@perfil ou link do TikTok")} autoComplete="off" />
        <span className="text-xs font-normal text-dim">{t("Dá pra adicionar mais depois.")}</span>
      </label>
      {error && <span role="alert" className="text-[13px] text-live">{error}</span>}
      <div className="flex justify-end gap-2.5">
        <Link to="/" className="btn btn-sec">
          {t("Voltar")}
        </Link>
        <Submit>{t("Criar e continuar")}</Submit>
      </div>
    </form>
  );
}

function StepStudy({ slug }: { slug: string }) {
  const qc = useQueryClient();
  const { data } = useQuery({ queryKey: keys.channel(slug), queryFn: () => commands.getChannel(slug) });
  const [error, action] = useActionState(async (_: string | null, fd: FormData) => {
    try {
      await commands.addInspiration(slug, String(fd.get("handle") ?? ""));
      await qc.invalidateQueries({ queryKey: keys.channel(slug) });
      return null;
    } catch (e) {
      return errMsg(e);
    }
  }, null);
  const list = data ? [data.ref_handle, ...data.inspirations].filter((h) => h && h !== "none") : [];
  return (
    <>
      <form action={action} className="flex flex-wrap items-end gap-2 px-5 pb-4">
        <label className="flex flex-[1_1_300px] flex-col gap-1.5 text-[13px] font-medium">
          {t("Adicionar canal")}
          <input type="text" name="handle" placeholder={t("@perfil ou link do TikTok")} className="w-full" autoComplete="off" />
        </label>
        <Submit>{t("Adicionar")}</Submit>
        {error && <span role="alert" className="basis-full text-xs text-live">{error}</span>}
      </form>
      {!data && <Skeleton className="mx-5 mb-4 h-10" />}
      {list.map((h, i) => (
        <div key={h} className="flex items-center gap-4 border-t border-line-soft px-5 py-3.5">
          <span className="mono flex-1 font-medium"><bdi>@{h}</bdi></span>
          <span className="text-[13px] text-muted">{i === 0 ? t("principal") : t("extra")}</span>
          <button
            className="icon-btn"
            type="button"
            aria-label={t("Remover @{h}", { h })}
            onClick={async () => {
              await commands.removeInspiration(slug, h).catch((e) => toast(errMsg(e), "error"));
              await qc.invalidateQueries({ queryKey: keys.channel(slug) });
            }}
          >
            <IX size={14} />
          </button>
        </div>
      ))}
    </>
  );
}

function StepAccounts({ slug }: { slug: string }) {
  const { data } = useQuery({ queryKey: keys.channel(slug), queryFn: () => commands.getChannel(slug) });
  if (!data) return <Skeleton className="mx-5 mb-4 h-16" />;
  return (
    <>
      <PlatformLogin key={`${slug}:tiktok`} slug={slug} platform="tiktok" handle={data.summary.tiktok} />
      <PlatformLogin key={`${slug}:youtube`} slug={slug} platform="youtube" handle={data.summary.youtube} />
    </>
  );
}

function StepSchedule({ slug }: { slug: string }) {
  const { data } = useQuery({ queryKey: keys.channel(slug), queryFn: () => commands.getChannel(slug) });
  if (!data) return <Skeleton className="mx-5 mb-4 h-40" />;
  return <SchedulePicker slug={slug} schedule={data.schedule} nextLabel={data.summary.next_post_label} />;
}

const HEAD: { title: MsgKey; sub: MsgKey }[] = [
  { title: "Nome e estilo", sub: "Só o básico. O agente descobre o resto estudando as referências." },
  { title: "Onde esse canal posta", sub: "Conecte pelo menos uma. Dá pra adicionar a outra depois." },
  { title: "Canais pra estudar", sub: "O agente baixa os vídeos desses perfis e aprende o estilo deles." },
  { title: "Quando postar", sub: "Escolha os dias e os horários. Dá pra mudar quando quiser." },
];

export default function NewChannel() {
  const [step, setStep] = useState(0);
  const [slug, setSlug] = useState<string | null>(null);
  const nav = useNavigate();
  const { data } = useQuery({
    queryKey: keys.channel(slug ?? ""),
    queryFn: () => commands.getChannel(slug!),
    enabled: !!slug,
  });
  const head = HEAD[step]!;
  const footer =
    step === 1 ? (data?.summary.tiktok ? t("TikTok pronto. Pode seguir e terminar o YouTube depois.") : t("Sem conta, o canal só faz vídeos de teste.")) : "";

  return (
    <div className="flex max-w-[1120px] flex-col gap-7">
      <header className="flex flex-col gap-1.5">
        <h1 className="m-0 text-2xl font-semibold tracking-[-0.02em]">{slug ? t("Novo canal: {slug}", { slug }) : t("Novo canal")}</h1>
        <p className="m-0 max-w-[70ch] text-muted">{t("Quatro passos: nome, contas, canais pra estudar e horários. Dá pra mudar tudo depois na página do canal.")}</p>
      </header>
      <Stepper at={step} />
      <div className="flex flex-wrap items-start gap-6">
        <Panel id="passo" title={t(head.title)} sub={t(head.sub)} className="min-w-0 flex-[3_1_540px]">
          <div key={step} className="anim-rise">
            {step === 0 && (
              <StepName
                onDone={(s) => {
                  setSlug(s);
                  setStep(1);
                }}
              />
            )}
            {step === 1 && slug && <StepAccounts slug={slug} />}
            {step === 2 && slug && <StepStudy slug={slug} />}
            {step === 3 && slug && <StepSchedule slug={slug} />}
          </div>
          {step > 0 && slug && (
            <div className="flex flex-wrap items-center gap-2.5 border-t border-line-soft px-5 py-4">
              <span className="flex-[1_1_240px] text-[13px] text-muted">{footer}</span>
              <button className="btn btn-sec" type="button" onClick={() => setStep(step - 1)} disabled={step === 1}>
                {t("Voltar")}
              </button>
              {step < 3 ? (
                <button className="btn btn-pri" type="button" onClick={() => setStep(step + 1)}>
                  {t("Continuar")}
                </button>
              ) : (
                <button
                  className="btn btn-pri"
                  type="button"
                  onClick={() => {
                    toast(t("{slug} criado", { slug }));
                    void nav({ to: "/canal/$slug", params: { slug } });
                  }}
                >
                  {t("Terminar")}
                </button>
              )}
            </div>
          )}
        </Panel>
        <div className="flex min-w-0 flex-[2_1_320px] flex-col gap-6">
          <Panel id="como" title={t("Como funciona")}>
            <div className="px-5 pb-4">
              {([
                "Você clica em conectar e abre uma janela do Edge que é só desse canal.",
                "Faz o login normal. Se pedir código, ele chega no seu celular ou email como sempre.",
                "O tkd percebe que entrou, guarda o @ da conta e a foto, e fecha a janela. Não precisa logar de novo.",
              ] as MsgKey[]).map((k, i) => (
                <div key={i} className="grid grid-cols-[28px_minmax(0,1fr)] items-start gap-3 border-t border-line-soft py-3 text-[13px] text-fg-2 first:border-t-0">
                  <b className="grid size-7 place-items-center rounded-lg bg-[#1f2125] text-xs font-semibold text-muted">{i + 1}</b>
                  <span>{t(k)}</span>
                </div>
              ))}
            </div>
          </Panel>
          <Panel id="seg" title={t("Por que uma janela por canal")}>
            <div className="flex flex-col gap-3 px-5 pb-5 text-[13px] text-fg-2">
              <div className="flex items-start gap-2.5">
                <IShield className="mt-0.5 flex-none text-muted" />
                <span>{t("As contas de um canal nunca se misturam com as de outro. Antes de postar, o tkd confere se está na conta certa.")}</span>
              </div>
              <div className="flex items-start gap-2.5">
                <ILock className="mt-0.5 flex-none text-muted" />
                <span>{t("Sua senha nunca passa pelo tkd. O login fica só no seu computador.")}</span>
              </div>
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}
