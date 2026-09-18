/**
 * 配对面板（UI 规格 §2.3）—— 由 `audiolink://pair-required` 事件驱动弹出。
 *
 * **两个方向共用一个事件**（契约只冻结了一个事件名），靠载荷里的 `pin` 区分：
 * - `pin` 非空 → 本机是**接收端**：对方要在它的屏幕上输入这个码 → 这里只负责把码亮出来；
 * - `pin` 为空 → 本机是**发起端**：请用户输入对方屏幕上显示的码 → 这里出输入框。
 *
 * 为什么用 `pin` 是否为空来区分，而不是加一个字段：契约 §6 的载荷形状是冻结的
 * （`{ idShort, name, pin }`），加字段等于改契约；而"有没有码要展示"本来就是这两种情形的全部差别。
 *
 * 不做：「记住此设备」勾选框 —— 信任落库由内核在配对成功时完成，UI 上没有对应入参，
 * 做成假开关比不做更糟。
 *
 * 本轮补上的两件事（原实现缺失，属于可用性缺陷而不是风格问题）：
 *  1. **Esc 关闭**：对话框能弹出来就必须能不靠鼠标退出去；
 *  2. **倒计时有长度**：60 秒的窗口原来只写在一行小字里，现在是刻度会走的进度条。
 */

import { useEffect, useState } from "react";

import { isPinWellFormed, type PairRequiredPayload } from "../types";
import { t } from "../i18n";

/** 配对码有效期。载荷里没有过期时刻，这里按协议常量（`docs/03-protocol.md` §5：60 s）本地倒计时。 */
const PIN_TTL_SECONDS = 60;

interface PairDialogProps {
  request: PairRequiredPayload;
  /** 上一次提交失败的原因（人话整句，来自引擎）；空串表示还没失败过。 */
  reason: string;
  /** 提交配对码；`idShort` 决定是哪条会话（可能同时有多个对端在配对）。 */
  onSubmit: (idShort: string, pin: string) => Promise<boolean>;
  onDismiss: () => void;
}

export function PairDialog({ request, reason, onSubmit, onDismiss }: PairDialogProps) {
  const [pin, setPin] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [remaining, setRemaining] = useState(PIN_TTL_SECONDS);

  const displayMode = request.pin !== "";

  // 新请求到达 → 清空输入并重新计时（同一个码只倒计时一次）
  useEffect(() => {
    setPin("");
    setRemaining(PIN_TTL_SECONDS);
    const timer = window.setInterval(() => {
      setRemaining((seconds) => (seconds > 0 ? seconds - 1 : 0));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [request.idShort, request.pin]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onDismiss();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onDismiss]);

  const expired = remaining === 0;
  const canSubmit = !displayMode && isPinWellFormed(pin) && !submitting && !expired;
  const ratio = Math.max(0, Math.min(1, remaining / PIN_TTL_SECONDS));

  const submit = async () => {
    if (!canSubmit) {
      return;
    }
    setSubmitting(true);
    try {
      await onSubmit(request.idShort, pin);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-chassis/85 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="pair-title"
        className="plate w-full max-w-md"
      >
        <header className="flex items-center gap-3 border-b border-line bg-bench px-4 py-3">
          <span className="lamp h-2.5 w-2.5" data-on={displayMode ? "busy" : "warn"} />
          <h2 id="pair-title" className="min-w-0 truncate t-lead font-semibold text-silk">
            {t("pair.title", { name: request.name })}
          </h2>
          <span className="num ml-auto shrink-0 t-cap text-silk-3">fp:{request.idShort}</span>
        </header>

        <div className="p-4">
          {displayMode ? (
            <>
              <p className="t-cap leading-relaxed text-silk-3">
                {t("pair.receive")}
              </p>
              {/* 大号等宽数字：要照着念、照着敲，字号与字距优先 */}
              <output
                aria-label={t("pair.code")}
                className="num well mt-3 block py-4 text-center text-[40px] leading-none tracking-[0.26em] text-silk"
              >
                {request.pin}
              </output>
            </>
          ) : (
            <>
              <p className="t-cap leading-relaxed text-silk-3">
                {t("pair.send")}
              </p>
              <label className="silk-sm mt-4 block" htmlFor="pair-pin">
                {t("pair.input")}
              </label>
              <input
                id="pair-pin"
                value={pin}
                // 只留数字：粘贴带空格/连字符的号码时也不至于直接判错
                onChange={(event) => setPin(event.target.value.replace(/\D/g, "").slice(0, 6))}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    void submit();
                  }
                }}
                inputMode="numeric"
                autoComplete="one-time-code"
                autoFocus
                maxLength={6}
                placeholder="______"
                className="num mt-1.5 h-14 w-full border border-line bg-chassis px-3 text-center text-[28px] tracking-[0.36em] text-silk placeholder:text-silk-3"
              />

              {reason === "" ? null : (
                <p role="alert" className="mt-2 t-cap text-ink-live">
                  {reason}
                </p>
              )}
            </>
          )}

          {/* 60 秒的窗口：刻度会走，快走完时转成播出灯的颜色 */}
          <div className="mt-4 h-1 w-full bg-line" aria-hidden="true">
            <div
              className={`h-full ${expired ? "bg-lamp-live" : "bg-lamp-warn"}`}
              style={{ width: `${ratio * 100}%` }}
            />
          </div>
          <p className="num mt-2 t-cap text-silk-3">
            {expired
              ? displayMode
                ? t("pair.expired_receive")
                : t("pair.expired_send")
              : t("pair.ttl", { seconds: remaining })}
          </p>

          {displayMode ? (
            <button
              type="button"
              onClick={onDismiss}
              className="key key-primary mt-4 h-11 w-full t-body"
            >
              {t("pair.got_it")}
            </button>
          ) : (
            <div className="mt-4 flex gap-2">
              <button
                type="button"
                onClick={onDismiss}
                className="key h-11 flex-1 t-body"
              >
                {t("pair.later")}
              </button>
              <button
                type="button"
                disabled={!canSubmit}
                onClick={() => void submit()}
                className="key key-primary h-11 flex-1 t-body"
              >
                {submitting ? t("pair.verifying") : t("pair.confirm")}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
