import { useEffect, useState } from "react";
import "./App.css";
import { TabBar, type TabId } from "./TabBar";
import { TodayScreen } from "./screens/TodayScreen";
import { CoalPoolScreen } from "./screens/CoalPoolScreen";
import { ContractScreen } from "./screens/ContractScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { MeScreen } from "./screens/MeScreen";
import { LoginScreen } from "./LoginScreen";
import { isLoggedIn } from "./storage";

function App() {
  const [tab, setTab] = useState<TabId>("today");
  const [authed, setAuthed] = useState(isLoggedIn());

  // 监听认证变化 (登录/登出后自动切屏)
  useEffect(() => {
    const onChange = () => setAuthed(isLoggedIn());
    window.addEventListener("doudou:auth_changed", onChange);
    return () => window.removeEventListener("doudou:auth_changed", onChange);
  }, []);

  if (!authed) {
    return <LoginScreen />;
  }

  return (
    <div className="app">
      <div className="app-content">
        {tab === "today" && <TodayScreen onNavigate={setTab} />}
        {tab === "pool" && <CoalPoolScreen />}
        {tab === "contract" && <ContractScreen />}
        {tab === "history" && <HistoryScreen />}
        {tab === "me" && <MeScreen />}
      </div>
      <TabBar active={tab} onChange={setTab} />
    </div>
  );
}

export default App;
