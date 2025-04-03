import React from "react";

import { BaseLayout, WelcomeLayout } from "./layout";
import { Welcome, Login, Help, Home, Convert } from "./modules";
import { routerFactory } from "./router";

export enum ROUTE {
  WELCOME,
  HELP,
  LOGIN,
  HOME,
  CONVERT,
}

interface ROUTE_DATA {
  [ROUTE.LOGIN]: {
    username: string;
  };
}

const layout = (
  Layout: React.FC<{ children: React.ReactNode }>,
  Component: React.FC,
) => (
  <Layout>
    <Component />
  </Layout>
);
const welcome = (Component: React.FC) => () => layout(WelcomeLayout, Component);
const base = (Component: React.FC) => () => layout(BaseLayout, Component);

const router = routerFactory<ROUTE, ROUTE_DATA>({
  [ROUTE.WELCOME]: welcome(Welcome),
  [ROUTE.LOGIN]: welcome(Login),
  [ROUTE.HELP]: base(Help),
  [ROUTE.HOME]: base(Home),
  [ROUTE.CONVERT]: base(Convert),
});

export const {
  Router,
  Redirect,
  useLocation,
  useNavigate,
  useRoute,
  useRouteData,
} = router;
