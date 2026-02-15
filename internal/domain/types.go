package domain

import "time"

// Priority levels for backlog items
type Priority string

const (
	// P0 is the highest priority.
	P0 Priority = "p0"
	// P1 is high priority.
	P1 Priority = "p1"
	// P2 is medium priority.
	P2 Priority = "p2"
	// P3 is lowest priority.
	P3 Priority = "p3"
)

// ItemType categorizes backlog items
type ItemType string

const (
	// Bug represents a bug report.
	Bug ItemType = "bug"
	// Feature represents a new feature request.
	Feature ItemType = "feature"
	// Task represents a general task.
	Task ItemType = "task"
	// Refactor represents a refactoring task.
	Refactor ItemType = "refactor"
	// Research represents a research task.
	Research ItemType = "research"
)

// Effort estimation
type Effort string

const (
	// Small indicates small effort.
	Small Effort = "s"
	// Medium indicates medium effort.
	Medium Effort = "m"
	// Large indicates large effort.
	Large Effort = "l"
	// ExtraLarge indicates extra large effort.
	ExtraLarge Effort = "xl"
)

// Horizon planning buckets
type Horizon string

const (
	// Now indicates immediate horizon.
	Now Horizon = "now"
	// Next indicates next sprint horizon.
	Next Horizon = "next"
	// Later indicates future horizon.
	Later Horizon = "later"
)

// Consensus strength between council members
type Consensus string

const (
	// Strong indicates strong consensus.
	Strong Consensus = "strong"
	// Moderate indicates moderate consensus.
	Moderate Consensus = "moderate"
	// Split indicates split/weak consensus.
	Split Consensus = "split"
)

// ContextQuality assessment
type ContextQuality string

const (
	// HighQuality indicates high context quality.
	HighQuality ContextQuality = "high"
	// MediumQuality indicates medium context quality.
	MediumQuality ContextQuality = "medium"
	// LowQuality indicates low context quality.
	LowQuality ContextQuality = "low"
)

// CouncilInput is what each council agent receives
type CouncilInput struct {
	Vision     string
	RepoState  RepoState
	HumanInput string
	Date       string
}

// RepoState captures repository context
type RepoState struct {
	RecentCommits []string
	OpenIssues    []string
	OpenPRs       []string
	FileTree      string
}

// CouncilOutput is what each council agent returns
type CouncilOutput struct {
	Councilor   string        `json:"councilor"`
	Perspective string        `json:"perspective"`
	Confidence  float64       `json:"confidence"`
	Summary     string        `json:"summary"`
	Items       []CouncilItem `json:"items"`
	Meta        CouncilMeta   `json:"meta"`
}

// CouncilItem is a single backlog item proposed by a council member
type CouncilItem struct {
	Title        string   `json:"title"`
	Priority     Priority `json:"priority"`
	Type         ItemType `json:"type"`
	Rationale    string   `json:"rationale"`
	Risk         string   `json:"risk"`
	Effort       Effort   `json:"effort"`
	Dependencies []string `json:"dependencies"`
	Evidence     string   `json:"evidence"`
}

// CouncilMeta has metadata about the council run
type CouncilMeta struct {
	ItemsProposed   int            `json:"items_proposed"`
	ContextQuality  ContextQuality `json:"context_quality"`
	VisionAlignment string         `json:"vision_alignment"`
}

// SynthesisResult is ORACLE's unified output
type SynthesisResult struct {
	Synthesizer string          `json:"synthesizer"`
	Model       string          `json:"model"`
	Summary     string          `json:"summary"`
	Items       []SynthesisItem `json:"items"`
	Conflicts   []Conflict      `json:"conflicts_resolved"`
	Dropped     []DroppedItem   `json:"dropped_items"`
}

// SynthesisItem is a final backlog item ready for issue creation
type SynthesisItem struct {
	Title           string         `json:"title"`
	Priority        Priority       `json:"priority"`
	Type            ItemType       `json:"type"`
	Horizon         Horizon        `json:"horizon"`
	Effort          Effort         `json:"effort"`
	Body            string         `json:"body"`
	Labels          []string       `json:"labels"`
	CouncilSupport  CouncilSupport `json:"council_support"`
	VisionAlignment string         `json:"vision_alignment"`
}

// CouncilSupport tracks which council members supported an item
type CouncilSupport struct {
	ProposedBy []string  `json:"proposed_by"`
	OpposedBy  []string  `json:"opposed_by"`
	Consensus  Consensus `json:"consensus"`
}

// Conflict records a resolved disagreement
type Conflict struct {
	Item         string `json:"item"`
	Disagreement string `json:"disagreement"`
	Resolution   string `json:"resolution"`
}

// DroppedItem is an item that didn't make the cut
type DroppedItem struct {
	Title  string `json:"title"`
	Reason string `json:"reason"`
}

// SpawnResult tracks outcome of running a single council agent
type SpawnResult struct {
	Output   *CouncilOutput
	Skipped  bool
	Error    error
	Model    string
	Retries  int
	Duration time.Duration
}

// CreatedIssue represents a GitHub issue that was created
type CreatedIssue struct {
	Number int
	URL    string
	Title  string
}
