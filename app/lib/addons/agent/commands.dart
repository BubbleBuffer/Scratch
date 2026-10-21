import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../registry/command_registry.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'dev_replay.dart';
import 'navigation.dart';

void registerAgentCommands(CommandRegistry r) {
  if (devReplayEnabled) {
    r.register(
      Command(
        id: 'agent.replay.live',
        label: 'Replay chat live…',
        icon: Ico.comment,
        shortcuts: const [
          SingleActivator(
            LogicalKeyboardKey.keyR,
            control: true,
            shift: true,
            alt: true,
          ),
        ],
        isEnabled: (context) {
          final app = AppScope.maybeOf(context);
          return app != null &&
              AgentStore.of(app).enabled &&
              AgentStore.of(app).host != AgentHost.hub;
        },
        run: showReplayCommand,
      ),
    );
  }
  r.register(
    Command(
      id: 'agent.open',
      label: 'Toggle agent panel',
      icon: Ico.comment,
      // Ctrl+J toggles the agent panel.
      shortcuts: const [
        SingleActivator(LogicalKeyboardKey.keyJ, control: true),
      ],
      isEnabled: (context) {
        final app = AppScope.maybeOf(context);
        return app != null && !context.mobile && AgentStore.of(app).enabled;
      },
      run: toggleAgentPanel,
    ),
  );
}
