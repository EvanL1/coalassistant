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
import { initializeCloudStorage } from "./cloudStorage";

function App() {
  const [tab, setTab] = useState<TabId>("today");
  const [authed, setAuthed] = useState<boolean | null>(null);
  const [admin, setAdmin] = useState(false);
  const [storageReady, setStorageReady] = useState(false);
  const [storageError, setStorageError] = useState<string | null>(null);

  // 监听认证变化 (登录/登出后自动切屏)
  useEffect(() => {
    let active = true;
    const refresh = async () => {
      const session = await getSession();
      if (!active) return;
      setAdmin(session.admin);
      setAuthed(session.authenticated);
    };
    const onChange = () => void refresh();
    window.addEventListener("doudou:auth_changed", onChange);
    void refresh();
    return () => {
      active = false;
      window.removeEventListener("doudou:auth_changed", onChange);
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
    return <LoginScreen />;
  }

  return (
    <div className="app">
      <TabBar active={tab} onChange={setTab} />
      <main className="app-content">
        <section className={`screen screen-${tab}`} aria-label="当前功能页面">
          {tab === "today" && (
            <>
              <IndexTicker />
              <TodayScreen onNavigate={setTab} />
            </>
          )}
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
