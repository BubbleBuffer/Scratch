---
type: topic
layer: implementation
summary: "The optional Board database layout and its live-query adapter."
parent: "[[addons/index]]"
status: current
---

# Board

`BoardAddon` in `app/lib/addons/board/api.dart` registers the Board `ViewType`, before Timeline in the layout menu. It declares `dependsOn: ['database']` and imports that add-on only through `database/api.dart`. It is optional in `shell/addons.dart`, enabled by default; Settings → About → Add-ons → Manage can disable it for the next start.

Files: `board_view.dart` preserves the database card presentation and layout options; `database_board_source.dart` adapts `CollectionStore` queries to `BoardSource`. Dragging, lazy columns, mobile long press and keyboard activation remain in the shared `app/lib/ui/board/` renderer. There is no new kanban note type.

The existing `app/test/board_renderer_test.dart`, database interaction and golden tests remain in place; only moved import paths change. `app/test/addons/addon_boot_test.dart` boots Board with Database alone and checks the layout order and optional disabling.
