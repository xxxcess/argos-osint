# Layouts

Ctrl+L cycles these. `/layout <name>` jumps. The name is the word in the
first column. The prompt band and the shortcut footer stay on every layout.
The prompt is bound to the canvas view, including the dashboard desk.

| Name | What you see |
| --- | --- |
| `classic` | Application launcher on the left, the active module and its chat stream on the right. This is the default. |
| `dashboard` | Launcher plus a gauge grid: CPU, memory, GPU, disk. |
| `tabs` | Module titles across the top, canvas and the event log side by side. |
| `modal` | The app-search card. Ctrl+P opens the same card over any layout. |
| `vertical` | Launcher, then the canvas stacked over the event log. |
| `horizontal` | Canvas on top, event log underneath. No launcher column. |
| `three` | Launcher, canvas, and a context rail (provider, brain, Gmail, host). |
| `float` | Canvas full width, with a quick-action card on the right. |
| `grid` | Nine widgets: CPU, memory, GPU, disk, provider, brain, Gmail, cases, reports. |
| `zen` | The open view, one host line, and a CPU sparkline. |

Focus order with Tab is launcher, canvas, prompt, when the layout has a
launcher. Otherwise Tab moves between the canvas and the prompt.

On the canvas, `j` / `k` scroll the stream. On the case desk, `J` / `K`
change which case the prompt is talking to. On Providers, Gmail, and
Settings, `j` / `k` move between fields and Enter edits the field. Esc
stops editing before it leaves the app.

The footer reads `[Ctrl+P] App Search` and `[Tab] Jump Focus | [Esc] Exit App`.
The prompt border carries the view name, modality, layout, and the database
label when the line fits. A narrow terminal keeps the view name and drops
the path.
