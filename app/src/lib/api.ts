// Ponte com o backend: comandos e tipos gerados (bindings.ts) com nomes curtos.
import type * as B from "../bindings";

export { commands } from "../bindings";

export type ChannelSummary = B.ChannelSummary_Serialize;
export type ChannelDetail = B.ChannelDetail_Serialize;
export type NowState = B.NowState_Serialize;
export type Stage = B.Stage;
export type ScheduleDto = B.ScheduleDto;
export type NoteDto = B.NoteDto;
export type TaskPick = B.TaskPick;
export type AgentConfig = B.AgentConfig;
export type ChannelAgent = B.ChannelAgent;
export type TaskRow = B.TaskRow;
// f64 vira `number | null` no TS (NaN); o backend nunca manda NaN aqui
type NoNull<T> = { [K in keyof T]: Exclude<T[K], null> };
export type UsageRow = NoNull<B.UsageRowDto>;
export type TaskCompareDto = NoNull<B.TaskCompareDto>;
export type UsageSummary = Omit<B.UsageSummary, "total" | "by_channel" | "by_task" | "by_pair" | "by_model" | "daily" | "before_after"> & {
  total: UsageRow;
  by_channel: UsageRow[];
  by_task: UsageRow[];
  by_pair: UsageRow[];
  by_model: UsageRow[];
  daily: NoNull<B.DayUsage>[];
  before_after: TaskCompareDto[];
};
export type EventDto = B.EventDto_Serialize;
export type LiveBatch = B.LiveBatch_Serialize;
export type BrowserStatus = B.BrowserStatus_Serialize;
export type AppStatus = B.AppStatus;
export type AppError = B.AppError;

/** Mensagem legivel de qualquer erro vindo do invoke. */
export function errMsg(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}
export type SetupItem = B.SetupItem_Serialize;
export type ClipDto = B.ClipDto_Serialize;
export type ClipsView = B.ClipsView_Serialize;
