export type Page = "chat" | "memory" | "modules" | "cookbook" | "settings";
export type ChatMetrics = {
  outputTokens?: number;
  promptTokens?: number;
  evalDurationMs?: number;
  totalDurationMs?: number;
  tokensPerSecond?: number;
};
export type Message = {
  id: string;
  role: "user" | "assistant";
  text: string;
  source?: "user" | "model" | "ui_preview";
  runId?: string;
  status?: "running" | "completed" | "cancelled" | "failed" | "interrupted";
  sources?: { path: string; title: string; excerpt: string }[];
  model?: string;
  thinking?: string;
  metrics?: ChatMetrics;
};
export type Session = {
  id: string;
  title: string;
  created: string;
  messages: Message[];
};
export type Note = {
  id: string;
  title: string;
  content: string;
  updated: string;
};
export type Activity = { id: string; text: string; time: string };
export type Preferences = {
  theme: "light" | "dark";
  provider: string;
  model: string;
  modules: string[];
};
export type State = {
  sessions: Session[];
  active: string;
  notes: Note[];
  activities: Activity[];
  preferences: Preferences;
};
export const STORAGE_KEY = "lim-harness-ui-v1";
export const newSession = (): Session => ({
  id: crypto.randomUUID(),
  title: "Phiên mới",
  created: new Date().toISOString(),
  messages: [],
});
export function initialState(): State {
  const session = newSession();
  return {
    sessions: [session],
    active: session.id,
    notes: [],
    activities: [],
    preferences: { theme: "dark", provider: "", model: "", modules: ["text"] },
  };
}
export function readState(): { state: State; error: string } {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { state: initialState(), error: "" };
    const data = JSON.parse(raw);
    const validString = (v: unknown) => typeof v === "string";
    const validDate = (v: unknown) =>
      typeof v === "string" && Number.isFinite(Date.parse(v));
    if (
      !data ||
      !Array.isArray(data.sessions) ||
      !data.sessions.length ||
      !data.sessions.every(
        (s: Session) =>
          s &&
          validString(s.id) &&
          validString(s.title) &&
          validDate(s.created) &&
          Array.isArray(s.messages) &&
          s.messages.every(
            (m) =>
              m &&
              validString(m.id) &&
              validString(m.text) &&
              ["user", "assistant"].includes(m.role),
          ),
      ) ||
      !data.sessions.some((s: Session) => s.id === data.active) ||
      !Array.isArray(data.notes) ||
      !data.notes.every(
        (n: Note) =>
          n &&
          validString(n.id) &&
          validString(n.title) &&
          validString(n.content) &&
          validDate(n.updated),
      ) ||
      !Array.isArray(data.activities) ||
      !data.activities.every(
        (a: Activity) =>
          a && validString(a.id) && validString(a.text) && validDate(a.time),
      ) ||
      !data.preferences ||
      !["light", "dark"].includes(data.preferences.theme) ||
      !validString(data.preferences.provider) ||
      !validString(data.preferences.model) ||
      !Array.isArray(data.preferences.modules) ||
      !data.preferences.modules.every(validString)
    )
      throw new Error("invalid state");
    return { state: data, error: "" };
  } catch {
    return {
      state: initialState(),
      error:
        "Không đọc được dữ liệu cục bộ. Bạn vẫn có thể thử giao diện; dữ liệu cũ chưa bị ghi đè.",
    };
  }
}
export const moduleCatalog = [
  {
    id: "text",
    name: "Xử lý văn bản",
    tag: "TEXT",
    description: "Tiền xử lý văn bản và chuẩn hóa đầu vào cho workflow.",
    icon: "text",
  },
  {
    id: "sentiment",
    name: "Phân tích sentiment",
    tag: "NLP",
    description: "Nhận diện sắc thái cảm xúc trong nội dung văn bản.",
    icon: "sentiment",
  },
  {
    id: "privacy",
    name: "Che thông tin",
    tag: "PRIVACY",
    description: "Phát hiện và che thông tin nhạy cảm trước khi xử lý.",
    icon: "privacy",
  },
  {
    id: "audio",
    name: "Nhận diện hợp âm",
    tag: "AUDIO",
    description: "Một ví dụ tích hợp model audio qua module contract.",
    icon: "audio",
  },
] as const;
