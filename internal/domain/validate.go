package domain

import (
	"fmt"
	"strings"
)

// Valid returns true if the priority is valid.
func (p Priority) Valid() bool {
	switch p {
	case P0, P1, P2, P3:
		return true
	}
	return false
}

// Valid returns true if the item type is valid.
func (t ItemType) Valid() bool {
	switch t {
	case Bug, Feature, Task, Refactor, Research:
		return true
	}
	return false
}

// Valid returns true if the effort is valid.
func (e Effort) Valid() bool {
	switch e {
	case Small, Medium, Large, ExtraLarge:
		return true
	}
	return false
}

// Valid returns true if the horizon is valid.
func (h Horizon) Valid() bool {
	switch h {
	case Now, Next, Later:
		return true
	}
	return false
}

// Validate checks the council output for errors.
func (c CouncilOutput) Validate() error {
	var errs []string
	if c.Councilor == "" {
		errs = append(errs, "councilor is required")
	}
	if c.Perspective == "" {
		errs = append(errs, "perspective is required")
	}
	if c.Confidence < 0 || c.Confidence > 1 {
		errs = append(errs, "confidence must be 0.0-1.0")
	}
	for i, item := range c.Items {
		if err := item.Validate(); err != nil {
			errs = append(errs, fmt.Sprintf("items[%d]: %s", i, err))
		}
	}
	if len(errs) > 0 {
		return fmt.Errorf("invalid council output: %s", strings.Join(errs, "; "))
	}
	return nil
}

// Validate checks the council item for errors.
func (item CouncilItem) Validate() error {
	var errs []string
	if item.Title == "" {
		errs = append(errs, "title is required")
	}
	if len(item.Title) > 200 {
		errs = append(errs, "title exceeds 200 characters")
	}
	if !item.Priority.Valid() {
		errs = append(errs, fmt.Sprintf("invalid priority %q", item.Priority))
	}
	if !item.Type.Valid() {
		errs = append(errs, fmt.Sprintf("invalid type %q", item.Type))
	}
	if !item.Effort.Valid() {
		errs = append(errs, fmt.Sprintf("invalid effort %q", item.Effort))
	}
	if len(errs) > 0 {
		return fmt.Errorf("%s", strings.Join(errs, "; "))
	}
	return nil
}

// Validate checks the synthesis result for errors.
func (s SynthesisResult) Validate() error {
	var errs []string
	for i, item := range s.Items {
		if err := item.Validate(); err != nil {
			errs = append(errs, fmt.Sprintf("items[%d]: %s", i, err))
		}
	}
	if len(errs) > 0 {
		return fmt.Errorf("invalid synthesis result: %s", strings.Join(errs, "; "))
	}
	return nil
}

// Validate checks the synthesis item for errors.
func (item SynthesisItem) Validate() error {
	var errs []string
	if item.Title == "" {
		errs = append(errs, "title is required")
	}
	if !item.Priority.Valid() {
		errs = append(errs, fmt.Sprintf("invalid priority %q", item.Priority))
	}
	if !item.Type.Valid() {
		errs = append(errs, fmt.Sprintf("invalid type %q", item.Type))
	}
	if !item.Horizon.Valid() {
		errs = append(errs, fmt.Sprintf("invalid horizon %q", item.Horizon))
	}
	if !item.Effort.Valid() {
		errs = append(errs, fmt.Sprintf("invalid effort %q", item.Effort))
	}
	if item.Body == "" {
		errs = append(errs, "body is required")
	}
	if len(errs) > 0 {
		return fmt.Errorf("%s", strings.Join(errs, "; "))
	}
	return nil
}
