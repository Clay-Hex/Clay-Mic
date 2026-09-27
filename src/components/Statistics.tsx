import { useEffect, useState } from "react";
import { tauriInvoke, tauriListen, type UsageStats } from "../lib/config";

export function Statistics() {
  const [stats, setStats] = useState<UsageStats | null>(null);

  // Initial fetch covers anything that changed while this page was not
  // mounted; after that every backend stats write arrives as an event.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    tauriInvoke<UsageStats>("get_stats")
      .then((payload) => {
        if (!disposed) setStats(payload);
      })
      .catch((error) => console.warn("[clay-mic] get_stats failed:", error));
    void tauriListen<UsageStats>("stats://updated", (payload) => {
      if (!disposed) setStats(payload);
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch((error) =>
        console.warn("[clay-mic] subscribe stats failed:", error),
      );
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const s = stats;

  return (
    <div className="max-w-2xl space-y-5">
      <div className="bg-white rounded-xl border border-gray-200 overflow-hidden">
        <div className="px-5 py-3.5 border-b border-gray-100">
          <div>
            <h3 className="text-[13px] font-semibold text-gray-800">使用统计</h3>
            <p className="text-[11px] text-gray-400 mt-0.5">
              语音次数与时长 · 实时更新
            </p>
          </div>
        </div>

        <div className="p-5 grid grid-cols-3 gap-3">
          <StatCard label="语音次数" value={fmt(s?.voice_sessions ?? 0)} unit="次" color="blue" />
          <StatCard label="总语音时长" value={fmtDuration(s?.total_voice_seconds ?? 0)} unit="" color="emerald" />
          <StatCard label="最长单次" value={fmtDuration(s?.longest_session_seconds ?? 0)} unit="" color="amber" />
        </div>
      </div>

      <div className="bg-white rounded-xl border border-gray-200 overflow-hidden">
        <div className="px-5 py-3.5 border-b border-gray-100">
          <h3 className="text-[13px] font-semibold text-gray-800">字符统计</h3>
          <p className="text-[11px] text-gray-400 mt-0.5">STT 输出 / LLM 输入与输出</p>
        </div>

        <div className="p-5 grid grid-cols-3 gap-3">
          <StatCard label="STT 输出" value={fmtChars(s?.stt_chars ?? 0)} unit="字符" color="blue" />
          <StatCard label="LLM 输入" value={fmtTokens(s?.llm_input_tokens ?? 0)} unit="tokens" color="emerald" />
          <StatCard label="LLM 输出" value={fmtTokens(s?.llm_output_tokens ?? 0)} unit="tokens" color="purple" />
        </div>
      </div>
    </div>
  );
}

function fmt(n: number): string {
  return n.toLocaleString();
}

function fmtDuration(secs: number): string {
  if (secs < 60) return `${Math.round(secs)}s`;
  if (secs < 3600) return `${(secs / 60).toFixed(1)}m`;
  return `${(secs / 3600).toFixed(1)}h`;
}

function fmtChars(n: number): string {
  if (n < 10_000) return n.toLocaleString();
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

function fmtTokens(n: number): string {
  if (n < 10_000) return n.toLocaleString();
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

const COLOR_MAP: Record<string, { bg: string; text: string }> = {
  blue: { bg: "bg-blue-50", text: "text-blue-600" },
  emerald: { bg: "bg-emerald-50", text: "text-emerald-600" },
  amber: { bg: "bg-amber-50", text: "text-amber-600" },
  purple: { bg: "bg-purple-50", text: "text-purple-600" },
};

function StatCard({
  label,
  value,
  unit,
  color,
}: {
  label: string;
  value: string;
  unit: string;
  color: string;
}) {
  const c = COLOR_MAP[color] || COLOR_MAP.blue;
  return (
    <div className={`${c.bg} rounded-lg p-3`}>
      <p className="text-[11px] text-gray-500 mb-1">{label}</p>
      <p className={`text-xl font-bold ${c.text}`}>
        {value}
        {unit && <span className="text-xs font-normal text-gray-400 ml-1">{unit}</span>}
      </p>
    </div>
  );
}
