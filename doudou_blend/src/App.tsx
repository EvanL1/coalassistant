import { useEffect, useState } from "react";
import "./App.css";
import { TabBar, type TabId } from "./TabBar";
import { TodayScreen } from "./screens/TodayScreen";
import { CoalPoolScreen } from "./screens/CoalPoolScreen";
import { ContractScreen } from "./screens/ContractScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { MeScreen } from "./screens/MeScreen";
import { LoginScreen } from "./LoginScreen";
import { getSession } from "./auth";
import { AdminScreen } from "./screens/AdminScreen";
import { IndexTicker } from "./IndexTicker";
import { initializeCloudStorage, resetSyncState } from "./cloudStorage";
import { SESSION_EXPIRED_EVENT, SESSION_EXPIRED_MESSAGE } from "./http";
import { SyncBanner } from "./SyncBanner";

function App() {
  const [tab, setTab] = useState<TabId>("today");
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [admin, setAdmin] = useState(false);
  const [storageReady, setStorageReady] = useState(false);
  const [storageError, setStorageError] = useState<string | null>(null);
  const [loginNotice, setLoginNotice] = useState<string | null>(null);

  // 监听认证变化 (登录/登出后自动切屏); 任一接口回 401 = 会话失效, 直接回登录页.
  useEffect(() => {
    let active = true;
    const refresh = async () => {
      const session = await getSession();
      if (!active) return;
      setAdmin(session.admin);
      setAuthed(session.authenticated);
      if (session.authenticated) setLoginNotice(null);
    };
    const onChange = () => void refresh();
    const onExpired = () => {
      // 会话失效后同步状态 (写回失败/冲突) 不再成立, 一并复位, 下次登录重新初始化.
      resetSyncState();
      setAdmin(false);
      setAuthed(false);
      setLoginNotice(SESSION_EXPIRED_MESSAGE);
    };
    window.addEventListener("doudou:auth_changed", onChange);
    window.addEventListener(SESSION_EXPIRED_EVENT, onExpired);
    void refresh();
    return () => {
      active = false;
      window.removeEventListener("doudou:auth_changed", onChange);
      window.removeEventListener(SESSION_EXPIRED_EVENT, onExpired);
    };
  }, []);

  useEffect(() => {
    let active = true;
    let stopSync: () => void = () => undefined;
    // 管理员不进五屏应用, 没有偏好/合同要同步.
    if (!authed || admin) {
      setStorageReady(false);
      setStorageError(null);
      return () => undefined;
    }

    setStorageReady(false);
    setStorageError(null);
    void initializeCloudStorage()
      .then((cleanup) => {
        if (!active) {
          cleanup();
          return;
        }
        stopSync = cleanup;
        setStorageReady(true);
      })
      .catch(() => {
        if (active) setStorageError("云端数据加载失败，请刷新重试");
      });

    return () => {
      active = false;
      stopSync();
    };
  }, [authed, admin]);

  if (authed && admin) {
    return <AdminScreen />;
  }

  if (authed == null || (authed && !storageReady)) {
    return (
      <div className="login-loading" role="status">
        {storageError ?? (authed ? "正在同步云端数据…" : "正在验证登录状态…")}
        {storageError && (
          <button className="btn btn-primary" onClick={() => location.reload()}>
            重新加载
          </button>
        )}
      </div>
    );
  }

  if (!authed) {
    return <LoginScreen notice={loginNotice} />;
  }

  return (
    <div className="app">
      <TabBar active={tab} onChange={setTab} />
      <main className="app-content">
        <section className={`screen screen-${tab}`} aria-label="当前功能页面">
          {/* 同步横幅与行情条装进同一个吸顶容器, 上下排开; 两个各自 top:0 会互相盖住. */}
          <div className="sticky-top">
            <SyncBanner />
            {(tab === "today" || tab === "pool") && <IndexTicker />}
          </div>
          {tab === "today" && <TodayScreen onNavigate={setTab} />}
          {tab === "pool" && <CoalPoolScreen />}
          {tab === "contract" && <ContractScreen />}
          {tab === "history" && <HistoryScreen />}
          {tab === "me" && <MeScreen />}
        </section>
      </main>
    </div>
  );
}

export default App;
