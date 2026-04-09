import { createRootRoute, Outlet } from "@tanstack/react-router";

export const Route = createRootRoute({
  component: RootLayout,
});

/**
 * Root shell. Routes own the viewport — no padding, no max-width, no
 * inline styles here. Layout decisions live inside each route's CSS file.
 */
function RootLayout() {
  return <Outlet />;
}
