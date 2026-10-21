// Settings > AI > Connect an agent: one command per agent (Claude Code and
// Codex), each with a Copy button. The command is run on the computer where
// that agent runs. It creates a token for the agent, adds the notes server to
// its config and uses the Notes desktop app's hub sign-in on that computer.
// This app writes no config files and makes no network calls here. Shown on
// every platform.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import '../settings/controls.dart';

class ConnectAgentSection extends StatelessWidget {
  const ConnectAgentSection({super.key, this.top = 0});

  /// Space above the heading. Settings > AI passes 24 when a section comes
  /// before it, as AgentTokensSection does.
  final double top;

  static const claudeCommand = 'notes setup claude';
  static const codexCommand = 'notes setup codex';

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      key: const ValueKey('connect-agent'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsHeading('Connect an agent', top: top),
        Text(
          "Run one command on the computer where the agent runs. It creates a token for that agent and adds the notes server to its config. It uses the Notes desktop app's hub sign-in on that computer.",
          style: tsRaw(13, 18, 400, c.textSecondary),
        ),
        const SizedBox(height: 12),
        SettingsRow(
          key: const ValueKey('connect-agent-row-claude'),
          label: 'Claude Code',
          stack: true,
          trailing: const CopyableText(
            claudeCommand,
            textKey: ValueKey('connect-agent-claude'),
          ),
        ),
        SettingsRow(
          key: const ValueKey('connect-agent-row-codex'),
          label: 'Codex',
          stack: true,
          trailing: const CopyableText(
            codexCommand,
            textKey: ValueKey('connect-agent-codex'),
          ),
        ),
      ],
    );
  }
}
