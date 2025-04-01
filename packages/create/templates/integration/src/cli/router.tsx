import React, {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { IsUnknown, Nullable } from "tsdef";

type AnyEnum = number | string;

interface RouterContextValue<
  Route extends AnyEnum,
  RoutesData extends Partial<Record<Route, unknown>>
> {
  route: Route;
  navigate: (<T extends Route>(
    to: T,
    ...[arg]: IsUnknown<RoutesData[T], [], [data: RoutesData[T]]>
  ) => void) & { back: () => void };
  data?: RoutesData[Route];
}

export type RoutesMap<Route extends AnyEnum> = Record<Route, () => JSX.Element>;

export interface RouterProps<Route> {
  children: React.ReactNode | React.ReactNode[];
  defaultRoute: Route;
  historyLimit?: number;
}

export interface RedirectProps<
  Route extends AnyEnum,
  RoutesData extends Partial<Record<Route, unknown>>
> {
  to: Route;
  data?: RoutesData[Route];
}

export const routerFactory = <
  Route extends AnyEnum,
  RoutesData extends Partial<Record<Route, unknown>>
>(
  routes: RoutesMap<Route>
) => {
  const RouterContext =
    createContext<Nullable<RouterContextValue<Route, RoutesData>>>(null);

  const Router: React.FC<RouterProps<Route>> = ({
    children,
    defaultRoute,
    historyLimit = 5,
  }) => {
    const [state, setState] = useState<{
      route: Route;
      data?: RoutesData[Route];
    }>({
      route: defaultRoute,
      data: undefined,
    });
    const history = useRef<
      {
        route: Route;
        data?: RoutesData[Route];
      }[]
    >([]);

    const navigate = <T extends Route>(to: T, data?: RoutesData[T]) => {
      history.current.push({ ...state });
      history.current = history.current.slice(-historyLimit);

      setState({
        route: to,
        data,
      });
    };

    navigate.back = () => {
      const prevRoute = history.current.pop();

      if (prevRoute) {
        setState({
          route: prevRoute.route,
          data: prevRoute.data,
        });
      }
    };

    return (
      <RouterContext.Provider
        value={{
          ...state,
          navigate,
        }}
      >
        {children}
      </RouterContext.Provider>
    );
  };

  const useRouterContext = () => {
    const ctx = useContext(RouterContext);

    if (!ctx) {
      throw new Error("RouterContext.Provider not found");
    }

    return ctx;
  };

  const useNavigate = () => useRouterContext().navigate;
  const useLocation = () => useRouterContext().route;
  const useRouteData = <T extends Route>() =>
    useRouterContext().data as RoutesData[T];
  const useRoute = () => {
    const location = useLocation();

    return routes[location]();
  };

  const Redirect = <T extends Route>({
    to,
    data,
  }: RedirectProps<T, RoutesData>) => {
    const navigate: any = useNavigate();

    useEffect(() => {
      navigate(to, data);
    }, []);

    return null;
  };

  return {
    Router,
    Redirect,
    useNavigate,
    useLocation,
    useRoute,
    useRouteData,
  };
};

export type RouterComponent<Route extends AnyEnum> = React.FC<
  RouterProps<Route>
>;
export type RedirectComponent<
  Route extends AnyEnum,
  RoutesData extends Partial<Record<Route, unknown>>
> = React.FC<RedirectProps<Route, RoutesData>>;
