import { createRootRoute, createRoute, createRouter, lazyRouteComponent } from "@tanstack/react-router";
import { Layout, PageSkeleton } from "./components/Layout";

// cada tela e carregada sob demanda
const root = createRootRoute({ component: Layout });
const channels = createRoute({ getParentRoute: () => root, path: "/", component: lazyRouteComponent(() => import("./routes/Channels")) });
const channel = createRoute({ getParentRoute: () => root, path: "/canal/$slug", component: lazyRouteComponent(() => import("./routes/Channel")) });
const newChannel = createRoute({ getParentRoute: () => root, path: "/novo", component: lazyRouteComponent(() => import("./routes/NewChannel")) });
const agents = createRoute({ getParentRoute: () => root, path: "/agentes", component: lazyRouteComponent(() => import("./routes/Agents")) });
const live = createRoute({ getParentRoute: () => root, path: "/ao-vivo", component: lazyRouteComponent(() => import("./routes/Live")) });
const clips = createRoute({ getParentRoute: () => root, path: "/cortes", component: lazyRouteComponent(() => import("./routes/Clips")) });
const setup = createRoute({ getParentRoute: () => root, path: "/primeiros-passos", component: lazyRouteComponent(() => import("./routes/Setup")) });
const usage = createRoute({ getParentRoute: () => root, path: "/consumo", component: lazyRouteComponent(() => import("./routes/Usage")) });

export const router = createRouter({
  routeTree: root.addChildren([channels, channel, newChannel, agents, live, clips, usage, setup]),
  defaultPendingComponent: PageSkeleton,
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
