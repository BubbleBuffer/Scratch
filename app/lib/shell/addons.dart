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
  const AddonEntry(this.addon, this.label, {this.optional = false});
  final Addon addon;
  final String label;
  final bool optional;
}

const addonList = [
  AddonEntry(LegacyAddon(), 'Built-ins'),
  AddonEntry(AgentAddon(), 'Agent', optional: true),
  AddonEntry(DocumentAddon(), 'Document'),
  AddonEntry(DatabaseAddon(), 'Database'),
  AddonEntry(BoardAddon(), 'Board', optional: true),
  AddonEntry(KanbanAddon(), 'Kanban', optional: true),
  AddonEntry(ScheduleAddon(), 'Schedules', optional: true),
  AddonEntry(TasksAddon(), 'Tasks', optional: true),
  AddonEntry(SearchCardAddon(), 'Search card', optional: true),
];

/// Ignore stale disabled ids and protect required add-ons. Reject an enabled
/// dependent whose dependency was disabled instead of silently booting it.
List<Addon> enabledAddons(Set<String> disabled) => [
  for (final entry in addonList)
    if (!entry.optional || !disabled.contains(entry.addon.id)) entry.addon,
];

AddonContext? _context;
AddonContext get appAddonContext {
  registerBuiltins();
  return _context!;
}

/// Compatibility name used by existing boot harnesses. Registration is once
/// per process; optional settings take effect at the next application start.
void registerBuiltins({Set<String> disabledAddons = const {}}) {
  if (_context != null) return;
  final ctx = AddonContext.global();
  bootAddons(enabledAddons(disabledAddons), ctx);
  _context = ctx;
  activeAddonContext = ctx;
}
