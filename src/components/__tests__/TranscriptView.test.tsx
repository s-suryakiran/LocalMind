import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TranscriptView } from "../TranscriptView";
import type { VoiceTranscript } from "../../lib/types";

const sample: VoiceTranscript = {
  id: "t1",
  sourcePath: "/tmp/a.wav",
  createdAt: 1,
  turns: [
    { speaker: 0, startMs: 0, endMs: 1000, text: "Hello there." },
    { speaker: 1, startMs: 1000, endMs: 2000, text: "Hi back." },
    { speaker: 0, startMs: 2000, endMs: 3000, text: "How are you?" },
  ],
};

describe("TranscriptView", () => {
  it("renders one bubble per turn", () => {
    render(<TranscriptView transcript={sample} />);
    expect(screen.getByText("Hello there.")).toBeInTheDocument();
    expect(screen.getByText("Hi back.")).toBeInTheDocument();
    expect(screen.getByText("How are you?")).toBeInTheDocument();
  });

  it("shows default speaker labels", () => {
    render(<TranscriptView transcript={sample} />);
    expect(screen.getAllByText(/Speaker 1/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText(/Speaker 2/).length).toBeGreaterThanOrEqual(1);
  });

  it("renames speaker via inline edit", () => {
    const lastNames: Record<number, string> = {};
    render(
      <TranscriptView
        transcript={sample}
        speakerNames={{ 0: "Surya" }}
        onRenameSpeaker={(id, name) => {
          lastNames[id] = name;
        }}
      />,
    );
    expect(screen.getAllByText("Surya").length).toBeGreaterThanOrEqual(1);

    const editBtn = screen.getAllByLabelText(/rename speaker 2/i)[0];
    fireEvent.click(editBtn);
    const input = screen.getByDisplayValue("Speaker 2");
    fireEvent.change(input, { target: { value: "Kavya" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(lastNames[1]).toBe("Kavya");
  });
});
