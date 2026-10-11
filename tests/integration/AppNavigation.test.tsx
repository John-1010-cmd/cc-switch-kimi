import { Suspense } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "@/App";
import { resetProviderState } from "../msw/state";
import { emitTauriEvent } from "../msw/tauriMocks";

// Keep the real App, SettingsPage and Tabs. A defaultTab prop alone cannot
// prove that reopening resets the already-mounted settings page's own tab.
vi.mock("@/components/usage/UsageDashboard", () => ({
  UsageDashboard: () => <div data-testid="usage-dashboard" />,
}));
vi.mock("@/components/providers/ProviderList", () => ({
  ProviderList: () => <div data-testid="providers-page" />,
}));
vi.mock("@/components/settings/LanguageSettings", () => ({
  LanguageSettings: () => <div data-testid="general-settings" />,
}));
vi.mock("@/components/FirstRunNoticeDialog", () => ({
  FirstRunNoticeDialog: () => null,
}));
vi.mock("@/components/settings/ThemeSettings", () => ({
  ThemeSettings: () => null,
}));
vi.mock("@/components/UpdateBadge", () => ({
  UpdateBadge: () => null,
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: async () => false,
    onResized: async () => () => {},
    setDecorations: async () => {},
  }),
}));

function renderApp() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Suspense fallback={null}>
        <App />
      </Suspense>
    </QueryClientProvider>,
  );
}

async function reopenApp() {
  await act(async () => {
    emitTauriEvent("navigate-to-usage", null);
  });
}

describe("default usage page navigation", () => {
  beforeEach(() => {
    resetProviderState();
    localStorage.clear();
  });

  it("cold-starts on the actual usage tab despite an old saved view", async () => {
    localStorage.setItem("cc-switch-last-view", "providers");
    renderApp();
    expect(await screen.findByTestId("usage-dashboard")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "usage.title" })).toHaveAttribute(
      "data-state",
      "active",
    );
  });

  it("returns from providers to usage when the running app is opened again", async () => {
    renderApp();
    await screen.findByTestId("usage-dashboard");
    fireEvent.keyDown(window, { key: "Escape" });
    await screen.findByTestId("providers-page");
    await reopenApp();
    expect(await screen.findByTestId("usage-dashboard")).toBeInTheDocument();
  });

  it("resets the real settings tab on every reopen, even if defaultTab is unchanged", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByTestId("usage-dashboard");
    for (let attempt = 0; attempt < 2; attempt += 1) {
      await user.click(
        screen.getByRole("tab", { name: "settings.tabGeneral" }),
      );
      await screen.findByTestId("general-settings");
      await reopenApp();
      expect(await screen.findByTestId("usage-dashboard")).toBeInTheDocument();
    }
  });

  it("does not reset navigation on ordinary window focus", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByTestId("usage-dashboard");
    await user.click(screen.getByRole("tab", { name: "settings.tabGeneral" }));
    await act(async () => {
      emitTauriEvent("tauri://focus", true);
    });
    expect(screen.getByTestId("general-settings")).toBeInTheDocument();
    expect(screen.queryByTestId("usage-dashboard")).not.toBeInTheDocument();
  });
});
