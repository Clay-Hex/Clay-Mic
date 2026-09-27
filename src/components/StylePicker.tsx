import { type ReactNode } from "react";
import { IndicatorVisual } from "../overlay/VoiceIndicator";
import "../overlay/overlay.css";
import "../overlay/indicator.css";
import "./StylePicker.css";

export interface StyleOption {
  value: string;
  label: string;
  preview: ReactNode;
}

export function StylePicker({
  options,
  value,
  onChange,
}: {
  options: StyleOption[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="style-picker" role="radiogroup">
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <div
            key={option.value}
            role="radio"
            aria-checked={selected}
            tabIndex={0}
            className={`style-tile${selected ? " is-selected" : ""}`}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => onChange(option.value)}
            onKeyDown={(event) => {
              if (event.key === " " || event.key === "Enter") {
                event.preventDefault();
                onChange(option.value);
              }
            }}
          >
            {option.preview}
            <span className="tile-label">
              <span className={`tile-radio${selected ? " is-on" : ""}`} />
              {option.label}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function OverlayPreview({ theme }: { theme: string }) {
  return (
    <div className="tile-shot tile-ov">
      <div className={`tile-scaler ov-theme-${theme}`}>
        <div className="ov-card">
          <div className="ov-title">
            <span>选择文本注入</span>
            <span className="ov-close">×</span>
          </div>
          <div className="ov-list">
            <div className="ov-item is-active">
              <div className="ov-text">会议纪要整理成待办</div>
            </div>
            <div className="ov-item">
              <div className="ov-text">接口文档周五前给到前端</div>
            </div>
          </div>
          <div className="ov-foot">↑↓ · Enter · Esc</div>
        </div>
      </div>
    </div>
  );
}

function IndicatorPreview({ style }: { style: string }) {
  return (
    <div className="tile-shot tile-ind">
      <div className="tile-scaler">
        <div className="thumb-ind ind-phase-recording">
          <div className="ind-panel">
            <IndicatorVisual style={style} />
            <div className="ind-label">正在聆听…</div>
          </div>
        </div>
      </div>
    </div>
  );
}

export const OVERLAY_STYLES: StyleOption[] = [
  { value: "aurora", label: "极光玻璃", preview: <OverlayPreview theme="aurora" /> },
  { value: "hud", label: "霓虹 HUD", preview: <OverlayPreview theme="hud" /> },
  {
    value: "deepspace",
    label: "深空流光",
    preview: <OverlayPreview theme="deepspace" />,
  },
  { value: "clean", label: "净白悬浮", preview: <OverlayPreview theme="clean" /> },
];

export const INDICATOR_STYLES: StyleOption[] = [
  {
    value: "remote-wave",
    label: "遥控器声波",
    preview: <IndicatorPreview style="remote-wave" />,
  },
  { value: "pulse", label: "脉冲麦克风", preview: <IndicatorPreview style="pulse" /> },
  {
    value: "spectrum",
    label: "频谱律动",
    preview: <IndicatorPreview style="spectrum" />,
  },
  { value: "orbit", label: "轨道光环", preview: <IndicatorPreview style="orbit" /> },
  {
    value: "particles",
    label: "粒子轨道",
    preview: <IndicatorPreview style="particles" />,
  },
  { value: "glow", label: "呼吸光晕", preview: <IndicatorPreview style="glow" /> },
];
