/**
 * 配对对话框（UI 规格 §2.3）—— 由 `audiolink://pair-required` 事件驱动弹出。
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
 */

import { useEffect, useState } from "react";

import { isPinWellFormed, type PairRequiredPayload } from "../types";

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

  const expired = remaining === 0;
  const canSubmit = !displayMode && isPinWellFormed(pin) && !submitting && !expired;

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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="pair-title"
        className="w-full max-w-sm rounded-xl border border-slate-200 bg-white p-5 shadow-xl dark:border-slate-800 dark:bg-slate-950"
      >
        <h2 id="pair-title" className="text-base font-medium">
          与「{request.name}」配对
        </h2>
        <p className="mt-1 font-mono text-xs text-slate-400">fp:{request.idShort}</p>

        {displayMode ? (
          <>
            <p className="mt-3 text-xs text-slate-500">
              请在对方的设备上输入下面这 6 位数字；两端显示一致才说明中间没有第三台设备。
            </p>
            {/* 大号等宽数字：要照着念、照着敲，字号与字距优先 */}
            <output
              className="mt-2 block rounded-lg bg-slate-50 py-3 text-center font-mono text-3xl tracking-[0.3em] tabular-nums dark:bg-slate-900"
              aria-label="配对码"
            >
              {request.pin}
            </output>
            <p className="mt-2 text-xs text-slate-400">
              {expired ? "已超过 60 秒，请让对方重新发起连接" : `有效期剩余 ${remaining} 秒`}
            </p>
            <button
              type="button"
              onClick={onDismiss}
              className="mt-4 h-9 w-full rounded-lg border border-slate-200 text-sm dark:border-slate-700"
            >
              知道了
            </button>
          </>
        ) : (
          <>
            <p className="mt-3 text-xs text-slate-500">
              对方要求配对。请输入对方设备屏幕上显示的 6 位数字。
            </p>
            <label className="mt-3 block text-xs text-slate-500" htmlFor="pair-pin">
              6 位配对码
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
              className="mt-1 h-10 w-full rounded-lg border border-slate-200 bg-transparent px-3 text-center font-mono text-lg tracking-[0.4em] tabular-nums dark:border-slate-700"
            />

            {reason === "" ? null : (
              <p role="alert" className="mt-2 text-xs text-red-600">
                {reason}
              </p>
            )}

            <p className="mt-2 text-xs text-slate-400">
              {expired ? "配对码已过期，请让对方重新发起连接" : `有效期剩余 ${remaining} 秒`}
            </p>

            <div className="mt-4 flex gap-2">
              <button
                type="button"
                onClick={onDismiss}
                className="h-9 flex-1 rounded-lg border border-slate-200 text-sm dark:border-slate-700"
              >
                稍后再说
              </button>
              <button
                type="button"
                disabled={!canSubmit}
                onClick={() => void submit()}
                className="h-9 flex-1 rounded-lg bg-indigo-600 text-sm font-medium text-white disabled:opacity-50"
              >
                {submitting ? "校验中…" : "确认配对"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
