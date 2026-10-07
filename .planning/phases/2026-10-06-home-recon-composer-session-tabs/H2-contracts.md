# H2 Contracts: Frozen Interface Decisions

## 1. Home Draft Key
- **Contract**: One dedicated Home new-investigation draft per Argos data/profile scope
- **Implementation**: Separate draft storage mechanism independent of investigation follow-up drafts
- **Symbol**: `HomeDraft` struct or enum variant in app state
- **Storage**: Reuse existing durable draft mechanism with special home-scoped key

## 2. Launch State Machine
- **States**: 
  - `Editable`: User typing in composer
  - `Accepting`: Submission in progress, validation running
  - `Accepted`: Investigation created, pending run referenced
  - `RecoverableFailure`: Validation/persistence failed, draft preserved
- **Transitions**: 
  - Editable → Accepting (on submit attempt)
  - Accepting → Accepted (on durable persistence success)
  - Accepting → RecoverableFailure (on validation/persistence failure)
  - RecoverableFailure → Editable (user fixes issue)
- **Late Acceptance**: Tied to captured draft revision token

## 3. Persistence Boundary
- **Reuse**: Current transaction/checkpoint path from existing draft persistence (`save_draft`)
- **Additional Requirement**: Submission token for duplicate prevention
- **Atomic Operation**: Investigation creation + first message + pending run + submission token
- **Fallback**: Smallest durable launch receipt if current mechanism insufficient

## 4. Tab Identity
- **Key**: Durable investigation ID (`Thread.id`)
- **Semantics**: Tab is a view into existing Recon history, not a copy
- **Operations**:
  - Close: Removes tab from open set, preserves investigation/jobs
  - Reopen: Restores tab if investigation still exists
  - Deduplication: Same investigation ID reuses existing tab

## 5. Navigation Intent
- **Tracking**: Last user navigation action during launch acceptance
- **Rule**: Delayed launch completion navigates only if user hasn't deliberately moved elsewhere
- **Fallback**: Quiet notification ("Investigation started — Open") if focus stolen

## 6. State Restoration (Per-Tab)
- **Owned by Tab**: 
  - Draft content and cursor position
  - Transcript anchor (scroll position)
  - Context-pane state (expanded/collapsed sections)
  - Unread watermark position
- **Owned Globally**: 
  - Application module state
  - Global UI settings (theme, layout preferences)
- **Avoid**: Storing same state in multiple places

## 7. Input Precedence Hierarchy
1. Completion/overlay (if active)
2. Focused control (composer, inputs, etc.)
3. App action (numeric shortcuts, module switches)
4. Eligible global command (palette, help, etc.)

## 8. Layout Measurements (Reference Points)
- **Baseline Title Row**: Measured from current `home_rows()` output pre-tab-strip
- **Apps Row**: Position after title/subtitle and upper spacing
- **Launch Block Bounds**: Total vertical space for composer + label + guidance
- **Footer Row**: Fixed 1-row footer
- **Minimum Blank-Row Reserve**: 
  - ≥4 rows at 32+ rows terminal height
  - ≥2 rows at 24-31 rows terminal height
  - Adaptive fallbacks for smaller heights

## Verification Approach
- H3 will implement these contracts with test seams
- H4 will verify launch vertical slice works
- State/crash tests will validate persistence boundaries