/**
 * 同步组域（M3 / FR-22）：组列表是**引擎侧的账本**，这里只做动作与回执，
 * UI 不自己缓存一份真相（每次操作后重新拉一次列表）。
 */

import { useCallback, useState } from "react";

import { api } from "../ipc";
import { toCommandError, type GroupView } from "../../types";
import { t } from "../../i18n";
import type { SetError, SetNotice } from "./shared";

export interface GroupsState {
  groups: GroupView[];
  groupBusy: boolean;
  refreshGroups: () => Promise<void>;
  createGroup: (idShorts: string[], leadMs: number) => Promise<void>;
  joinGroup: (idShort: string, groupId: number) => Promise<void>;
  leaveGroup: (idShort: string, groupId: number) => Promise<void>;
}

export function useGroupsState(setError: SetError, setNotice: SetNotice): GroupsState {
  const [groups, setGroups] = useState<GroupView[]>([]);
  const [groupBusy, setGroupBusy] = useState(false);

  /** 刷新同步组列表（M3）：操作后由调用方刷新，UI 不自己缓存真相。 */
  const refreshGroups = useCallback(async (): Promise<void> => {
    try {
      setGroups(await api.listGroups());
    } catch (raw) {
      setError(toCommandError(raw));
    }
  }, [setError]);

  /** 建组：把点名的对端组成临时同步组（提前量给排播留余量）。 */
  const createGroup = useCallback(
    async (idShorts: string[], leadMs: number): Promise<void> => {
      if (idShorts.length === 0) {
        setNotice(t("toast.pick_peer"));
        return;
      }
      setGroupBusy(true);
      try {
        const groupId = await api.createGroup(idShorts, leadMs);
        setNotice(t("toast.group_created", { id: groupId, count: idShorts.length }));
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshGroups();
        setGroupBusy(false);
      }
    },
    [refreshGroups, setError, setNotice],
  );

  /** 成员动态加入：运行中的其它成员不受影响（§7 / FR-22）。 */
  const joinGroup = useCallback(
    async (idShort: string, groupId: number): Promise<void> => {
      setGroupBusy(true);
      try {
        await api.joinGroup(idShort, groupId);
        setNotice(t("toast.group_joined", { peer: idShort, id: groupId }));
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshGroups();
        setGroupBusy(false);
      }
    },
    [refreshGroups, setError, setNotice],
  );

  /** 成员退出：组空了引擎会自己清掉条目。 */
  const leaveGroup = useCallback(
    async (idShort: string, groupId: number): Promise<void> => {
      setGroupBusy(true);
      try {
        await api.leaveGroup(idShort, groupId);
        setNotice(t("toast.group_left", { peer: idShort, id: groupId }));
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshGroups();
        setGroupBusy(false);
      }
    },
    [refreshGroups, setError, setNotice],
  );

  return { groups, groupBusy, refreshGroups, createGroup, joinGroup, leaveGroup };
}
