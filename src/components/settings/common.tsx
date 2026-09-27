import type { DownloadProgress } from "../../lib/config";

export function Field({
  label,
  help,
  action,
  children,
}: {
  label: string;
  help?: React.ReactNode;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-4">
      <label className="w-40 shrink-0 pt-2 text-[13px] text-gray-700 font-medium">
        {label}
      </label>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <div className="flex-1 min-w-0">{children}</div>
          {action && <div className="shrink-0">{action}</div>}
        </div>
        {help && (
          <p className="text-[12px] text-gray-400 leading-relaxed mt-1.5">
            {help}
          </p>
        )}
      </div>
    </div>
  );
}

export function Group({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-3">
      <p className="text-[14px] font-semibold text-gray-800">{label}</p>
      {children}
    </div>
  );
}

export function MetaRow({
  label,
  value,
  tone,
}: {
  label: string;
  value: React.ReactNode;
  tone?: "ok" | "warn";
}) {
  const valueClass =
    tone === "ok"
      ? "text-emerald-600"
      : tone === "warn"
        ? "text-amber-600"
        : "text-gray-700";
  return (
    <div className="flex items-baseline justify-between gap-3 text-[12px]">
      <span className="text-gray-500 shrink-0">{label}</span>
      <span className={`font-medium text-right ${valueClass}`}>{value}</span>
    </div>
  );
}

export function Note({
  children,
  tone,
}: {
  children: React.ReactNode;
  tone?: "warn";
}) {
  return (
    <p
      className={`text-[12px] leading-relaxed ${
        tone === "warn" ? "text-amber-600" : "text-gray-400"
      }`}
    >
      {children}
    </p>
  );
}

export function Message({
  children,
  tone = "muted",
}: {
  children: React.ReactNode;
  tone?: "muted" | "warn";
}) {
  return (
    <p
      className={`text-[12px] break-all ${
        tone === "warn" ? "text-amber-600" : "text-gray-500"
      }`}
    >
      {children}
    </p>
  );
}

export function SectionBox({ children }: { children: React.ReactNode }) {
  return (
    <div className="bg-white rounded-xl border border-gray-200 px-5 py-4 space-y-5">
      {children}
    </div>
  );
}

export function DownloadBar({ progress }: { progress: DownloadProgress }) {
  const label =
    progress.source === "binary"
      ? "运行时"
      : progress.source === "model"
        ? "模型"
        : "驱动";
  return (
    <div className="space-y-1.5">
      <div className="w-full h-2 bg-gray-100 rounded-full overflow-hidden">
        <div
          className="h-full bg-blue-500 rounded-full transition-all duration-300"
          style={{ width: `${progress.percent}%` }}
        />
      </div>
      <p className="text-[12px] text-gray-400">
        {label}：
        {progress.phase === "downloading"
          ? `下载中 ${progress.percent}%`
          : "解压中…"}
      </p>
    </div>
  );
}
