import { useEffect, useState } from "react";
import "./App.css";
import { TabBar, type TabId } from "./TabBar";
import { TodayScreen } from "./screens/TodayScreen";
import { CoalPoolScreen } from "./screens/CoalPoolScreen";
import { ContractScreen } from "./screens/ContractScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { MeScreen } from "./screens/MeScreen";
import { LoginScreen } from "./LoginScreen";
import { isLoggedIn } from "./auth";
import { IndexTicker } from "./IndexTicker";

function App() {
  const [tab, setTab] = useState<TabId>("today");
  const [authed, setAuthed] = useState<boolean | null>(null);

  // 监听认证变化 (登录/登出后自动切屏)
  useEffect(() => {
    let active = true;
    const refresh = async () => {
      const loggedIn = await isLoggedIn();
      if (active) setAuthed(loggedIn);
    };
    const onChange = () => void refresh();
    window.addEventListener("doudou:auth_changed", onChange);
    void refresh();
    return () => {
      active = false;
      window.removeEventListener("doudou:auth_changed", onChange);
    };
  }, []);

  if (authed == null) {
    return (
      <div className="login-loading" role="status">
        正在验证登录状态…
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
