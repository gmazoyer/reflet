import { Routes, Route } from "react-router-dom";
import Layout from "./components/Layout";
import { useEventStream } from "./hooks/useEventStream";
import Dashboard from "./pages/Dashboard";
import PeerList from "./pages/PeerList";
import PeerRoutes from "./pages/PeerRoutes";
import Lookup from "./pages/Lookup";

export default function App() {
  useEventStream();

  return (
    <Layout>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/peers" element={<PeerList />} />
        <Route path="/peers/:id/routes" element={<PeerRoutes />} />
        <Route path="/lookup" element={<Lookup />} />
      </Routes>
    </Layout>
  );
}
