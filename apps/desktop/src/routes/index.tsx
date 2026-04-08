import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: IndexPage,
});

function IndexPage() {
  return (
    <section>
      <h1 style={{ margin: 0 }}>Lantern</h1>
      <p style={{ color: "#666", marginTop: "0.25rem" }}>v0.0.1 — scaffold</p>
      <p style={{ marginTop: "2rem" }}>
        This is the empty Lantern desktop shell. Real UI lands in plan 1f.
      </p>
    </section>
  );
}
