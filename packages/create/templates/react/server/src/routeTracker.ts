import express, { Application, RequestHandler } from "express";

interface RouteInfo {
  method: string;
  path: string;
  description?: string;
  params?: string[];
}

export class ExpressWithRouteTracking {
  private app: Application;
  private routes: RouteInfo[] = [];

  constructor() {
    this.app = express();
  }

  // Extract parameters from a route path
  private extractParams(path: string): string[] {
    const paramMatches = path.match(/:([^\/]+)/g);
    return paramMatches ? paramMatches.map((param) => param.substring(1)) : [];
  }

  // Track a route when it's registered
  private trackRoute(method: string, path: string, description?: string) {
    const params = this.extractParams(path);
    this.routes.push({
      method: method.toUpperCase(),
      path,
      description,
      params: params.length > 0 ? params : undefined,
    });
  }

  // Wrapper methods for HTTP verbs
  get(path: string, ...handlers: RequestHandler[]) {
    this.trackRoute("GET", path);
    return this.app.get(path, ...handlers);
  }

  post(path: string, ...handlers: RequestHandler[]) {
    this.trackRoute("POST", path);
    return this.app.post(path, ...handlers);
  }

  put(path: string, ...handlers: RequestHandler[]) {
    this.trackRoute("PUT", path);
    return this.app.put(path, ...handlers);
  }

  delete(path: string, ...handlers: RequestHandler[]) {
    this.trackRoute("DELETE", path);
    return this.app.delete(path, ...handlers);
  }

  patch(path: string, ...handlers: RequestHandler[]) {
    this.trackRoute("PATCH", path);
    return this.app.patch(path, ...handlers);
  }

  // Get all tracked routes
  getRoutes(): RouteInfo[] {
    return [...this.routes];
  }

  // Get routes as JSON response
  getRoutesResponse() {
    return {
      message: "Available routes on this server",
      routes: this.getRoutes(),
      totalRoutes: this.routes.length,
    };
  }

  // Proxy all other Express methods
  use(...args: any[]) {
    // If first argument is a string (route path), track it
    if (typeof args[0] === "string") {
      const path = args[0];
      this.trackRoute("ALL", path, "Middleware route");
    }
    return this.app.use(...args);
  }

  listen(port: number, callback?: () => void) {
    return this.app.listen(port, callback);
  }

  // Get the underlying Express app if needed
  getApp(): Application {
    return this.app;
  }
}
