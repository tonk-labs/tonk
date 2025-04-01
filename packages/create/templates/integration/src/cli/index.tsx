import React from "react";
import { render } from "ink";

import { App } from "./app";
import { ROUTE, Router } from "./routes";

const cli = () => {
  render(
    <Router defaultRoute={ROUTE.WELCOME}>
      <App />
    </Router>
  );
};

export default cli;
