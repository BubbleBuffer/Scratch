// The fixed compiled-in list. Only the shell imports add-on entry points.
import '../addons/agent/api.dart';
import '../addons/board/api.dart';
import '../addons/database/api.dart';
import '../addons/document/api.dart';
import '../addons/kanban/api.dart';
import '../addons/schedule/api.dart';
import '../addons/legacy/api.dart';
import '../addons/search_card/api.dart';
import '../addons/tasks/api.dart';
import '../registry/addon_registry.dart';

class AddonEntry {
  const AddonEntry(this.addon, this.label);
  final Addon addon;
  final String label;
}

const addonList = [
  AddonEntry(LegacyAddon(), 'Built-ins'),
  AddonEntry(AgentAddon(), 'Agent'),
  AddonEntry(DocumentAddon(), 'Document'),
  AddonEntry(DatabaseAddon(), 'Database'),
  AddonEntry(BoardAddon(), 'Board'),
  AddonEntry(KanbanAddon(), 'Kanban'),
  AddonEntry(ScheduleAddon(), 'Schedules'),
  AddonEntry(TasksAddon(), 'Tasks'),
  AddonEntry(SearchCardAddon(), 'Search card'),
];

AddonContext? _context;
AddonContext get appAddonContext {
  registerBuiltins();
  return _context!;
}

/// Register every compiled-in add-on once per process.
void registerBuiltins() {
  if (_context != null) return;
  final ctx = AddonContext.global();
  bootAddons(addonList.map((entry) => entry.addon), ctx);
  _context = ctx;
  activeAddonContext = ctx;
}
