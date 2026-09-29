/**
 * 采集设备域：枚举、选择、当前正在采集的设备，以及 `canStartCapture`。
 *
 * `locked` 由调用方算：它还要看推流状态与对端列表（停止中 / 正在 start-stop / 已有对端在推流），
 * 那些不属于采集域。锁定中不许换设备，也要重读一次"当前正在采集哪台"。
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../ipc";
import { toCommandError, type CaptureDeviceView, type CommandError } from "../../types";
import type { SetError } from "./shared";

export interface CaptureState {
  captureDevices: CaptureDeviceView[];
  captureLoading: boolean;
  captureError: CommandError | null;
  selectedCaptureId: string;
  activeCapture: CaptureDeviceView | null;
  canStartCapture: boolean;
  selectCapture: (id: string) => void;
  refreshCaptureDevices: () => Promise<void>;
  refreshActiveCapture: () => Promise<void>;
}

export function useCaptureState(locked: boolean, onError: SetError): CaptureState {
  const [captureDevices, setCaptureDevices] = useState<CaptureDeviceView[]>([]);
  const [captureLoading, setCaptureLoading] = useState(true);
  const [captureError, setCaptureError] = useState<CommandError | null>(null);
  const [selectedCaptureId, setSelectedCaptureId] = useState("");
  const [activeCapture, setActiveCapture] = useState<CaptureDeviceView | null>(null);
  const captureRequest = useRef(0);
  const activeRequest = useRef(0);
  const selectedCapture = captureDevices.find((device) => selectedCaptureId === "" ? device.isDefault : device.id === selectedCaptureId);
  const canStartCapture = !captureLoading && captureError === null && selectedCapture !== undefined && selectedCapture.unavailableReason === null;

  const refreshCaptureDevices = useCallback(async () => {
    const request = ++captureRequest.current;
    setCaptureLoading(true);
    setCaptureError(null);
    try {
      const devices = await api.listCaptureDevices();
      if (request === captureRequest.current) setCaptureDevices(devices);
    } catch (raw) {
      if (request === captureRequest.current) setCaptureError(toCommandError(raw));
    } finally {
      if (request === captureRequest.current) setCaptureLoading(false);
    }
  }, []);

  const refreshActiveCapture = useCallback(async () => {
    const request = ++activeRequest.current;
    try {
      const device = await api.activeCaptureDevice();
      if (request === activeRequest.current) setActiveCapture(device);
    } catch (raw) {
      if (request === activeRequest.current) onError(toCommandError(raw));
    }
  }, [onError]);

  useEffect(() => {
    void refreshCaptureDevices();
    return () => { captureRequest.current += 1; activeRequest.current += 1; };
  }, [refreshCaptureDevices]);

  useEffect(() => {
    void refreshActiveCapture();
  }, [locked, refreshActiveCapture]);

  return {
    captureDevices, captureLoading, captureError, selectedCaptureId, activeCapture,
    canStartCapture, refreshCaptureDevices, refreshActiveCapture,
    // 保持内联：这个动作只在没锁定时生效，写成 useCallback 会让调用方拿到一个"稳定但不新鲜"的闭包。
    selectCapture: (id) => { if (!locked) setSelectedCaptureId(id); },
  };
}
