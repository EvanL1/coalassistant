import { useEffect, useState } from "react";
import "./App.css";
import { TabBar, type TabId } from "./TabBar";
import { TodayScreen } from "./screens/TodayScreen";
import { CoalPoolScreen } from "./screens/CoalPoolScreen";
import { ContractScreen } from "./screens/ContractScreen";
import { HistoryScreen } from "./screens/HistoryScreen";
import { MeScreen } from "./screens/MeScreen";
import { CustomersScreen } from "./screens/CustomersScreen";
import { QuotesScreen } from "./screens/QuotesScreen";
import { LoginScreen } from "./LoginScreen";
import {
  isLoggedIn,
  refreshCustomers,
  refreshQuotes,
  refreshSettings,
  refreshUserCoals,
} from "./storage";

function App() {
  const [tab, setTab] = useState<TabId>("today");
  const [authed, setAuthed] = useState(isLoggedIn());

  // 监听认证变化 (登录/登出后自动切屏)
  useEffect(() => {
    const onChange = () => setAuthed(isLoggedIn());
    window.addEventListener("doudou:auth_changed", onChange);
    return () => window.removeEventListener("doudou:auth_changed", onChange);
  }, []);

  // 启动时已登录: 后台拉 D1 最新数据 (新设备打开应用就能看到其他设备改的)
  useEffect(() => {
    if (authed) {
      void refreshUserCoals();
      void refreshSettings();
      void refreshCustomers();
      void refreshQuotes();
    }
  }, [authed]);

  if (!authed) {
    return <LoginScreen />;
  }

  return (
    <div className="app">
      <div className="app-content">
        {tab === "today" && <TodayScreen />}
        {tab === "customers" && <CustomersScreen />}
        {tab === "quotes" && <QuotesScreen />}
        {tab === "pool" && <CoalPoolScreen />}
        {tab === "contract" && <ContractScreen onBack={() => setTab("me")} />}
        {tab === "history" && <HistoryScreen onBack={() => setTab("me")} />}
        {tab === "me" && <MeScreen onNavigate={setTab} />}
      </div>
      <TabBar active={tab} onChange={setTab} />
    </div>
  );
}

export default App;
