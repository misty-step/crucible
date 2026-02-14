package domain

import "context"

// Agent evaluates a project from a specific perspective
type Agent interface {
	Perspective() string
	Evaluate(ctx context.Context, input CouncilInput) (*CouncilOutput, error)
}

// Synthesizer reconciles council outputs into a unified backlog
type Synthesizer interface {
	Synthesize(ctx context.Context, input SynthesisInput) (*SynthesisResult, error)
}

// SynthesisInput bundles everything the synthesizer needs
type SynthesisInput struct {
	CouncilOutputs []CouncilOutput
	Vision         string
	RepoContext    string
}

// Emitter outputs backlog items to an external system
type Emitter interface {
	Emit(ctx context.Context, items []SynthesisItem) ([]CreatedIssue, error)
}
