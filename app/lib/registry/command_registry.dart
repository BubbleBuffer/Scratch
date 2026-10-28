// The command registry: app-wide actions with an id, a label, an optional
// icon and keyboard shortcuts. The shell binds the shortcuts of every
// registered command (desktop); menus and the command palette
// (lib/shell/command_palette.dart) list them from here. Built-in commands
// live in lib/shell/commands.dart.

import 'package:flutter/widgets.dart';

import '../ui/ui.dart';

class Command {
  const Command({
    required this.id,
    required this.label,
    required this.run,
    this.icon,
    this.shortcuts = const [],
    this.isEnabled,
    this.disabledReason,
  });

  /// Stable id ("page.new").
  final String id;
  final String label;
  final Ico? icon;

  /// Desktop keyboard shortcuts.
  final List<SingleActivator> shortcuts;

  /// Runs the command; [context] is inside the app's navigator.
  final void Function(BuildContext context) run;
  final bool Function(BuildContext context)? isEnabled;

  /// What the palette says next to the command while [isEnabled] is false.
  final String? disabledReason;
}

class CommandRegistry {
  final _commands = <String, Command>{};

  void register(Command c) => _commands[c.id] = c;

  Command? of(String id) => _commands[id];

  Iterable<Command> get all => _commands.values;

  /// Runs [id] if it is registered and enabled.
  bool run(String id, BuildContext context) {
    final c = _commands[id];
    if (c == null || !(c.isEnabled?.call(context) ?? true)) return false;
    c.run(context);
    return true;
  }

  /// Shortcut bindings for [context] (for CallbackShortcuts).
  Map<ShortcutActivator, VoidCallback> bindings(BuildContext context) => {
        for (final c in _commands.values)
          for (final s in c.shortcuts) s: () => run(c.id, context),
      };
}

/// The app's commands. Creating it points [AppTooltip]'s shortcut hints at it.
final commandRegistry = _showShortcutsInTooltips(CommandRegistry());

CommandRegistry _showShortcutsInTooltips(CommandRegistry registry) {
  AppTooltip.commandShortcuts = (id) => registry.of(id)?.shortcuts;
  return registry;
}
