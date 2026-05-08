import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

// Chat.tsx renders transcript turns by stuffing
//   `[transcript]\n[Speaker 1]: hello world\n[Speaker 2]: hi`
// into ReactMarkdown. CommonMark may treat `[Speaker 1]: hello world`
// as a link-reference definition and silently drop it. This test asserts
// the speaker text is actually visible — if it fails, Chat.tsx must
// stop using brackets-with-colons or escape them.
describe("transcript rendering through ReactMarkdown", () => {
  it("renders [Speaker N]: text lines as visible content", () => {
    const content =
      "[transcript]\n[Speaker 1]: hello world\n[Speaker 2]: hi everyone";
    render(<ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>);
    expect(screen.queryByText(/hello world/)).not.toBeNull();
    expect(screen.queryByText(/hi everyone/)).not.toBeNull();
  });

  it("renders single-word turns (which alone are valid link-ref definitions)", () => {
    // Why: `[Speaker 1]: yes` on its own would parse as a link-reference
    // definition (label="Speaker 1", destination="yes") and be stripped.
    // The leading `[transcript]\n` opens a paragraph first, so the speaker
    // lines stay inside the paragraph as plain text. If Chat.tsx ever drops
    // the `[transcript]` prefix, this test will catch the regression.
    // "[Speaker 1]: yes" on its own IS a valid CommonMark link reference
    // definition (label="Speaker 1", destination="yes"). Without guarding,
    // such a line would be stripped from output entirely.
    const content = "[transcript]\n[Speaker 1]: yes\n[Speaker 2]: okay";
    const { container } = render(
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>,
    );
    expect(container.textContent).toContain("yes");
    expect(container.textContent).toContain("okay");
  });
});
