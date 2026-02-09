import { BrowserRouter, Routes, Route } from 'react-router';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import DashboardLayout from '@/layouts/DashboardLayout';
import Dashboard from '@/pages/Dashboard';
import Applications from '@/pages/Applications';
import Resumes from '@/pages/Resumes';
import Interviews from '@/pages/Interviews';
import JobDiscovery from '@/pages/JobDiscovery';
import Notifications from '@/pages/Notifications';
import Scheduler from '@/pages/Scheduler';

const queryClient = new QueryClient();

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Routes>
          <Route element={<DashboardLayout />}>
            <Route index element={<Dashboard />} />
            <Route path="applications" element={<Applications />} />
            <Route path="resumes" element={<Resumes />} />
            <Route path="interviews" element={<Interviews />} />
            <Route path="discovery" element={<JobDiscovery />} />
            <Route path="notifications" element={<Notifications />} />
            <Route path="scheduler" element={<Scheduler />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
