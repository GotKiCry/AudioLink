/**
 * 子 hook 之间共享的**写入口与桥类型**。
 *
 * 只放跨域的东西，规矩是：**读状态一律走参数，写状态只给 setter；两个域互相需要对方的结论时走桥。**
 * 单独成文件是为了让 `state/` 下的 hook 彼此不互相 import —— 依赖方向保持在
 * `shared → reductions → 各域 → ../useAudioLink`，不出现环。
 */

import type { RefObject } from "react";

import type { CommandError, PeerView } from "../../types";

/** `useState` 的 setter 收窄成"只写值"（setter 本身稳定，可安全进 deps）。 */
export type SetError = (error: CommandError | null) => void;

/** 同上，回执。 */
export type SetNotice = (notice: string | null) => void;

/**
 * 推流侧要读的**采集门控**（桥 1）。
 *
 * 采集与推流互相需要对方的结论（推流要知道"能不能开采集"，采集要知道"是不是锁定中"），
 * 两个 hook 没法互相要求"先声明"。于是采集侧把派生值写进这个 ref，
 * 推流侧只在**事件回调**里读它 —— 不进 deps，也就不会形成调用顺序环。
 */
export interface SendGate {
  canStartCapture: boolean;
  selectedCaptureId: string;
  refreshActiveCapture: () => Promise<void>;
}

/**
 * 事件订阅里要触发的**后置域**动作（桥 2）。
 *
 * 订阅在挂载时注册一次，闭包会一直指向首帧；所以传 ref 而不是函数本身，
 * 由 `useAudioLink` 里的镜像 effect 每次渲染后写入最新实现。
 */
export type PeersTap = RefObject<(next: PeerView[]) => void>;

/** 同上：`audiolink://groups` 到达时拉一次最新组列表。 */
export type GroupsTap = RefObject<() => void>;

/**
 * 自动推流决策要用的动作与记账 ref（桥 3）。
 *
 * 由推流域（`state/broadcast.ts`）产出、交给 `state/autoStart.ts` 使用：
 * 决策的**条件**横跨三个域，但**动作**属于推流，所以只把必要的动作与 ref 递过去，
 * 不把推流状态机整个暴露出去。
 */
export interface AutoStartParts {
  requestSend: (idShort: string) => Promise<void>;
  pausedRef: RefObject<boolean>;
  activeSendRef: RefObject<string | null>;
  autoStartedRef: RefObject<Set<string>>;
}
