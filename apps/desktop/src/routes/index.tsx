import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import {
  AddressDisplay,
  AmountInput,
  Button,
  WarningBanner,
} from "@lantern/ui";

export const Route = createFileRoute("/")({
  component: IndexPage,
});

/**
 * Dev sandbox route — exists to prove that @lantern/ui primitives import and
 * render cleanly inside the Tauri shell. This route will be replaced by the
 * real Home screen once feature routes come online.
 */
function IndexPage() {
  const [amount, setAmount] = useState("");

  return (
    <section>
      <h1>Lantern</h1>
      <p>Shared-UI sandbox — verifies @lantern/ui wires into the Tauri shell.</p>

      <WarningBanner severity="warning" title="Testnet mode">
        You are connected to the CKB testnet. Amounts are in Fibt and have no
        real value.
      </WarningBanner>

      <AddressDisplay
        label="Receive address"
        address="ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqvwg2cen8extgq8s5puft8vf40px3f599cytcyd8"
        copyable
        onCopy={(full) => {
          // Intentionally not calling navigator.clipboard here — this is a
          // pure UI sandbox. Real copy goes through a feature hook later.
          void full;
        }}
      />

      <AmountInput
        value={amount}
        onChange={setAmount}
        unit="Fibt"
        decimals={8}
        balance="1234.56789012"
        onMax={() => setAmount("1234.56789012")}
      />

      <Button variant="primary" size="md">
        Send
      </Button>
      <Button variant="secondary" size="md">
        Cancel
      </Button>
    </section>
  );
}
