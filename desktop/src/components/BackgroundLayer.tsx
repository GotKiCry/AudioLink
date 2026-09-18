/**
 * z0 应用背景 + z1 Mica 等效：玻璃之所以成立的「可透之物」。
 *
 * 这两层是**兄弟层**，不是应用壳（.al-shell）的自身背景，理由有两条，都是实测过的：
 *   1. 它们在 .al-shell 的层叠上下文里就会被壳压住 —— .al-shell 自己带 z-index，
 *      而带 z-index 的子元素（z0/z1）会盖住没有定位的普通流内容（侧栏、卡片全都消失）；
 *   2. 卡片 / chrome 的 backdrop-filter 采样域必须是「整张壁纸 + Mica 结果」，
 *      而不是某个祖先盒子的内部。
 * 所以它们 portal 到 document.body，与应用壳平级（DESIGN.md 红线 1 的同一原理）。
 *
 * config = null 表示用户还没配过：这时不写任何内联变量，交给 index.css 的令牌按
 * 主题给默认值（深色对角三色 / 浅色对角双色）。
 */

import { useEffect } from "react";
import { createPortal } from "react-dom";

import { applyWall } from "../lib/background";
import type { BackgroundConfig } from "../types";

export function BackgroundLayer({ config }: { config: BackgroundConfig | null }) {
  useEffect(() => {
    applyWall(config);
  }, [config]);

  return createPortal(
    <>
      <div className="al-desktop" aria-hidden="true" />
      <div className="al-mica" aria-hidden="true" />
    </>,
    document.body,
  );
}
