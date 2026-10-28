// The shell's built-in commands. Their shortcuts are bound by the shell.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../feedback/feedback_host.dart';
import '../registry/command_registry.dart';
import '../state/app_state.dart';
import '../state/tree_store.dart';
import '../ui/ui.dart';
import 'command_palette.dart';
import 'navigation.dart';
import 'search.dart';

void registerShellCommands(CommandRegistry r) {
  r.register(
    Command(
      id: 'page.new',
      label: 'New page',
      icon: Ico.compose,
      shortcuts: const [
        SingleActivator(LogicalKeyboardKey.keyN, control: true),
      ],
      run: (context) => createPage(context),
    ),
  );
  r.register(
    Command(
      id: 'search',
      label: 'Search',
      icon: Ico.search,
      // Ctrl+K reaches here unless the editor has a text selection (then it
      // adds a link).
      shortcuts: const [
        SingleActivator(LogicalKeyboardKey.keyK, control: true),
        SingleActivator(LogicalKeyboardKey.keyP, control: true),
      ],
      run: (context) {
        if (context.mobile) {
          goHome(context);
          searchRequest.value++;
        } else {
          showSearchOverlay(context);
        }
      },
    ),
  );
  r.register(
    Command(
      id: paletteCommandId,
      label: 'Command palette',
      icon: Ico.terminal,
      // Ctrl+K and Ctrl+P open search, and Ctrl+Shift+P is the usual key for
      // a command palette.
      shortcuts: const [
        SingleActivator(LogicalKeyboardKey.keyP, control: true, shift: true),
      ],
      run: (context) => openCommandPalette(context, registry: r),
    ),
  );
  r.register(
    Command(
      id: 'sidebar.toggle',
      label: 'Toggle sidebar',
      icon: Ico.sidebar,
      shortcuts: const [
        SingleActivator(LogicalKeyboardKey.backslash, control: true),
      ],
      run: (context) => context.read<TreeStore>().toggleSidebar(),
    ),
  );
  r.register(Command(id: 'home', label: 'Home', icon: Ico.home, run: goHome));
  r.register(
    Command(
      id: 'feedback.send',
      label: 'Send feedback',
      shortcuts: const [
        SingleActivator(LogicalKeyboardKey.keyF, control: true, shift: true),
      ],
      run: (context) => FeedbackHost.of(context).open(context),
    ),
  );
}
