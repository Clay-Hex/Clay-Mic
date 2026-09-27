import { useState } from "react";

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);

const NAMED_KEYS: Record<string, string> = {
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Escape: "Escape",
  CapsLock: "CapsLock",
  NumLock: "NumLock",
};

function modifiersOf(event: React.KeyboardEvent): string[] {
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.shiftKey) modifiers.push("Shift");
  if (event.altKey) modifiers.push("Alt");
  if (event.metaKey) modifiers.push("Super");
  return modifiers;
}

/** Maps a keydown to a token the backend hotkey parser understands. */
function keyToken(event: React.KeyboardEvent): string | null {
  const key = event.key;
  if (MODIFIER_KEYS.has(key)) return null;
  if (key === " ") return "Space";
  if (key.length === 1) return key.toUpperCase();
  if (key in NAMED_KEYS) return NAMED_KEYS[key];
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key;
  return null;
}

export function HotkeyInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [draft, setDraft] = useState("");

  const handleKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!recording) return;
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      setRecording(false);
      setDraft("");
      return;
    }
    if (
      (event.key === "Backspace" || event.key === "Delete") &&
      modifiersOf(event).length === 0
    ) {
      onChange("");
      setRecording(false);
      setDraft("");
      return;
    }

    const modifiers = modifiersOf(event);
    const token = keyToken(event);
    if (!token) {
      setDraft(modifiers.join("+"));
      return;
    }
    onChange([...modifiers, token].join("+"));
    setRecording(false);
    setDraft("");
  };

  return (
    <button
      type="button"
      className="input text-left"
      onFocus={() => setRecording(true)}
      onBlur={() => {
        setRecording(false);
        setDraft("");
      }}
      onKeyDown={handleKeyDown}
    >
      {recording ? draft || "按下快捷键…" : value || "未设置"}
    </button>
  );
}
