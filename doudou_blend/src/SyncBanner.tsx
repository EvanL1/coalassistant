import { useEffect, useState } from "react";
import {
  getSyncState,
  resolveSyncConflict,
  retrySync,
  SYNC_STATUS_EVENT,
} from "./cloudStorage";

/** 云端同步的常驻提示: 写回失败给重试; 云端被别人改过时让用户二选一. */
export function SyncBanner() {
  const [state, setState] = useState(getSyncState);
  // 二选一的请求在途: 两个按钮都不可点, 免得先后点了两个、最后本机与云端对不上.
  const [resolving, setResolving] = useState(false);

  useEffect(() => {
    const refresh = () => setState(getSyncState());
    window.addEventListener(SYNC_STATUS_EVENT, refresh);
    refresh();
    return () => window.removeEventListener(SYNC_STATUS_EVENT, refresh);
  }, []);

  async function resolve(choice: "local" | "cloud") {
    setResolving(true);
    try {
      await resolveSyncConflict(choice);
    } finally {
      setResolving(false);
    }
  }

  if (state.status === "conflict") {
    return (
      <div className="sync-banner" role="alert">
        <div className="sync-banner-text">
          <strong>云端数据在别的设备上改过了</strong>
          <span>
            本机还有没同步的改动。选「用云端」会放弃本机所有没同步的改动 (包括现在接着做的);
            选「用本机」会覆盖云端, 另一台设备的改动会丢。没选之前改动只存在本机。
            {state.error ? ` ${state.error}` : ""}
          </span>
        </div>
        <button
          className="btn btn-secondary"
          disabled={resolving}
          onClick={() => void resolve("cloud")}
        >
          用云端
        </button>
        <button
          className="btn btn-secondary"
          disabled={resolving}
          onClick={() => void resolve("local")}
        >
          用本机
        </button>
      </div>
    );
  }
  if (state.status !== "error") return null;
  return (
    <div className="sync-banner" role="alert">
      <div className="sync-banner-text">
        <strong>有改动还没同步到云端</strong>
        <span>{state.error ? `${state.error}。` : ""}改动保存在本机, 同步成功前不要换设备或退出登录。</span>
      </div>
      <button className="btn btn-secondary" onClick={retrySync}>
        重试
      </button>
    </div>
  );
}
