import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { OfflineBanner } from "../OfflineBanner";
import { useApp } from "../../lib/store";

describe("OfflineBanner", () => {
  beforeEach(() => {
    // Reset store between tests
    useApp.setState({ online: true, lastOnlineAt: Date.now() });
  });

  it("does not render when online", () => {
    useApp.setState({ online: true });
    render(<OfflineBanner />);
    expect(screen.queryByText(/host offline/i)).not.toBeInTheDocument();
  });

  it("renders when offline", () => {
    useApp.setState({ online: false });
    render(<OfflineBanner />);
    expect(screen.getByText(/host offline/i)).toBeInTheDocument();
  });
});
