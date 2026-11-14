# ARGOS // OSINT Native Workstation UI Specification
**Version:** 2.8.4-darwin-arm64  
**Design System:** Obsidian Charcoal Liquid Metal  
**Target Environment:** Tauri / React Desktop Wrapper (macOS / Linux arm64/x86_64)

---

## 1. Global Aesthetic & Design Philosophy

The interface is engineered to emulate a high-performance native desktop application designed for open-source intelligence (OSINT) analysts, automated wire ingestion pipelines, and multi-provider LLM orchestration. 

### 1.1 Color Palette & Surfaces
- **Background Obsidian (`#060709`):** The absolute grounding canvas across the viewport.
- **Charcoal Panels (`#0B0C0F` to `#111317`):** Elevated cards, modals, and container surfaces featuring subtle top-border highlights (`rgba(247, 245, 238, 0.09)`).
- **Warm Ivory (`#F7F5EE` / `#D6D2C4` / `#B8B4A8`):** Primary typography, headers, and liquid metal highlights. Light gray tones are strictly excluded to preserve deep contrast.
- **Liquid Metal Textures:**
  - *Interactive Dark Surface (`.liquid-metal-surface`):* Radial gradient overlay transitioning from ambient ivory sheen (`14%` opacity) into obsidian charcoal, complete on hover with an animated light-sheen sweep (`transform: translateX(110%)`).
  - *Ivory Action Button (`.liquid-metal-ivory-btn`):* Multi-stop radial gradient from crisp white (`#FFFFFF`) to warm ivory (`#F7F5EE`) and muted gold-ivory (`#B8B2A2`) with deep specular drop shadows.

---

## 2. Layout Architecture & Viewport Structure

The application is locked to the viewport height (`h-screen max-h-screen overflow-hidden`) and partitioned into three distinct vertical zones:

```
+-------------------------------------------------------------------------+
| WINDOW TITLE BAR: Traffic Light Dots | Title | Setup Gateway Button     |
+-------------------------------------------------------------------------+
| SIDEBAR HEAD:      | MAIN DISPLAY STAGE (Independently Scrollable)      |
| GW Status & URL    |                                                    |
|                    |  1. Workstation Status & Gateway Identity Widget   |
| NAV ITEMS:         |  (3-Column Grid + Full-Width Active Workspace)     |
| - DB Seed & Train  |                                                    |
| - News Cycles      |  2. Scoped Active Control View                     |
| - Brain Search     |     - DB Seeding & Rust-BERT Training              |
| - OSINT Tools      |     - News Cycle Ingestion                         |
| - Model Roles      |     - Brain Search (Actor Dossiers)                |
| - Jobs & Logs      |     - OSINT Tools & API Key Vault                  |
|                    |     - Model Assignments & Provider Table           |
|                    |     - Job Queue & Stdout Log Viewer                |
|                    |     - Workspace Intelligence Briefs                |
+--------------------+----------------------------------------------------+
```

### 2.1 Native Window Title Bar (`h-11`)
- **Left:** macOS-style traffic light control dots (`bg-rose-500`, `bg-amber-500`, `bg-emerald-500`) followed by the application title `ARGOS // WORKSTATION CONTROL HUB` and active breadcrumb indicator.
- **Right:** Quick action button to re-run the 2-step onboarding connection wizard.

### 2.2 Collapsible Left Workstation Context Sidebar (`aside`)
- **Width:** Toggleable between `w-72` (Expanded: full titles, subtitles, icons, live status metrics) and `w-20` (Collapsed: icon-only menu bar with native hover tooltips).
- **Sidebar Header:** Displays the live emerald `GW CONNECTED (14ms)` indicator signal and the gateway endpoint URL (`https://gw-node01.argos-osint.internal:8443`). Clicking this header repaints the main display area exclusively with the **Workstation Status & Gateway Telemetry Identity Widget**.
- **Navigation List:** Independently scrollable container (`independent-scroll`) housing the 6 primary workstation control views. Clicking any item repaints the main display with its associated control component.

---

## 3. Onboarding Wizard (2-Step Slideshow)

Prior to entering the main hub, unconfigured sessions present a modal wizard:
- **Step 1 (Gateway URL):** Centered card with an input field taking the API Gateway URL (placeholder hint: `https://gw-node01.argos-osint.internal:8443`).
- **Step 2 (Connectivity Check):** Compact 3-item checklist verifying API Gateway latency, Rust-BERT service handshake, and Model Roles catalog sync before launching the hub.

---

## 4. Workstation Status & Gateway Identity Widget

Rendered at the top of the main display stage when `activeView === 'status_identity'`:
1. **Top Row (3 Columns):**
   - *Native Client Shell:* Tauri / Rust UI platform (`macOS 15.2 // arm64`), workstation IP (`10.42.0.18`), token file reference, and silicon spec (`Apple M3 Max`).
   - *Gateway Backend Host:* Linux host OS (`x86_64`), gateway endpoint IP (`10.42.0.1:8443`), Rust-BERT co-processor (`RTX 6000 Ada / CUDA`), and uptime.
   - *Host Resource Telemetry:* Live-polling progress bars for CPU (`64c EPYC`), Memory (GB allocated vs total), and NVMe RAID Disk.

---

## 5. Scoped Control Views & Components

### 5.1 DB Seed & Train (`db_training`)
- **PostgreSQL Seeders:** Triggers for Consolidated Sanctions & Maritime IMO Registry (`55,910 Entities`), Dual-Use HS Tariff Taxonomy, and HNSW Vector Index builds.
- **Rust-BERT Classifier Trainer:** Displays active safetensors weights (`rust-bert-admission-v4.2`), validation F1 score (`0.962`), and trigger actions for fine-tuning epochs.

### 5.2 News Cycles (`news_cycles`)
- **Controls:** Button to trigger new cycle ingestion stamped with UTC date-time.
- **Cycle List:** Cards showing ingested raw wire documents, Rust-BERT admissions, Atlas leads extracted, and delete actions.

### 5.3 Brain Search (`brain_search`)
- **Catalog Search:** Instant filter input querying actor dossiers by designation (e.g., `APT-42 // Velvet Mirage`), region, or tags.
- **Dossier Inspector:** Detailed view displaying the `brain_summary` LLM role output, corroboration confidence score (`94%`), linked graph entities, and verified claims.

### 5.4 OSINT Tools & Key Vault (`tools_ctl`)
- **Tool Cards:** Grid of external providers and internal IPC services (`Shodan`, `Tavily`, `Censys`, `Maltego`, `Nvidia NIM`, `Rust-BERT`, `RSS Firehose`).
- **API Key Provisioning Modal:** Clicking disabled tools requiring an API key opens a secure modal prompt to vault the secret token.

### 5.5 Model Assignments (`model_assignments`)
- **Provider Table:** Lists `OpenRouter`, `Google`, and `Nvidia NIM` with connection status and inline key configuration options.
- **Pipeline Role Table:** Configures independent roles (`recon_agent`, `recon_decisions`, `recon_analyst`, `atlas_evaluator`, `atlas_analyst`, `brain_summary`) with provider selection, model routing, reasoning effort (`minimal` to `high`), variant, and fallback lists.

### 5.6 Job Statuses & Logs (`jobs_logs`)
- **Queue List:** Active and completed asynchronous worker jobs (`NEWS_CYCLE_INGEST`, `RUST_BERT_CLASSIFIER_TRAIN`, `DB_SEED_CORPUS`).
- **Terminal View:** Monospace stdout/stderr stream viewer with line numbering and status color coding.

---

## 6. Status & Category Color Semantics

To maintain operational clarity, semantic colors are strictly enforced across all widgets:
- **Emerald (`text-emerald-400` / `bg-emerald-950/50`):** Healthy quorums, online gateway handshakes, key-configured status, verified claims, and successful job exits.
- **Amber (`text-amber-400` / `bg-amber-950/60`):** Running tasks, pending verification, missing API keys, and warning states.
- **Cyan (`text-cyan-300` / `bg-cyan-950/50`):** CPU load metrics, client platform tags, active workspace IDs, and telemetry highlights.
- **Violet (`text-violet-300` / `bg-violet-950/50`):** Rust-BERT private co-processor service badges, memory utilization, and classification models.
- **Rose (`text-rose-300` / `bg-rose-950/40`):** Critical actor dossier tags, high-risk flags, and window close controls.