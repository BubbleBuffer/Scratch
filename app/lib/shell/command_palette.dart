// The command palette, Scratch's side. bb_ui draws it (package:bb_ui/palette.dart:
// fuzzy filter, keyboard, focus restore) over neutral commands; this maps the
// command registry onto them and runs the one the user picks. The commands'
// BuildContext actions, shortcut bindings and enabled checks stay in the
// registry: the palette only lists a command, shows its shortcut, and asks the
// registry to run it. Ctrl+Shift+P opens it (`palette.open`).

import 'package:bb_ui/palette.dart';
import 'package:flutter/widgets.dart';

import '../registry/command_registry.dart';
import '../ui/ui.dart';

/// The command that opens the palette. The palette does not list itself.
const paletteCommandId = 'palette.open';

/// What the palette says next to a disabled command that gave no reason.
const paletteDisabledFallback = 'Not available now';

/// The commands of [registry] as the palette lists them, by label. Enabled
/// checks run against [context] now, so a command that cannot run is listed
/// as disabled with its reason.
List<PaletteCommand> paletteCommands(
  CommandRegistry registry,
  BuildContext context,
) {
  final commands = [
    for (final c in registry.all)
      if (c.id != paletteCommandId) c,
  ]..sort((a, b) => a.label.toLowerCase().compareTo(b.label.toLowerCase()));
  return [
    for (final c in commands)
      _paletteCommand(c, enabled: c.isEnabled?.call(context) ?? true),
  ];
}

PaletteCommand _paletteCommand(Command c, {required bool enabled}) {
  return PaletteCommand(
    id: c.id,
    title: c.label,
    icon: c.icon,
    shortcutLabel: c.shortcuts.isEmpty ? null : shortcutText(c.shortcuts.first),
    enabled: enabled,
    disabledReason: enabled
        ? null
        : (c.disabledReason ?? paletteDisabledFallback),
    rowKey: ValueKey('palette-command-${c.id}'),
  );
}

bool _paletteOpen = false;

/// Opens the palette over [context]'s navigator and runs the command the user
/// picks through [registry] (the app's by default), after the palette is gone
/// and focus is back, so a command that opens something keeps its focus.
Future<void> openCommandPalette(
  BuildContext context, {
  CommandRegistry? registry,
}) async {
  if (_paletteOpen) return;
  _paletteOpen = true;
  final commands = registry ?? commandRegistry;
  try {
    final picked = await showCommandPalette(
      context,
      commands: paletteCommands(commands, context),
    );
    if (picked != null && context.mounted) commands.run(picked.id, context);
  } finally {
    _paletteOpen = false;
  }
}
