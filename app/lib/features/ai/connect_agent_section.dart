// Settings > Connectors > Apps that use your vault > Quick setup: one
// command per agent (Claude Code and Codex), each with a Copy button. The
// command is run on the computer where that agent runs. It creates a token
// for the agent, adds the notes server to its config and uses the Scratch
// desktop app's hub sign-in on that computer. This app writes no config
// files and makes no network calls here. Desktop only: Connectors leaves it
// out on phones.

import 'package:flutter/widgets.dart';

import '../settings/settings_kit.dart';

class ConnectAgentSection extends StatelessWidget {
  const ConnectAgentSection({super.key, this.first = false});

  /// Drops the section's top margin.
  final bool first;

  static const claudeCommand = 'notes setup claude';
  static const codexCommand = 'notes setup codex';

  @override
  Widget build(BuildContext context) {
    return KeyedSubtree(
      key: const ValueKey('connect-agent'),
      child: SSection(
        first: first,
        title: 'Quick setup',
        intro:
            'Run one command in a terminal on the computer where the app runs. It adds Scratch to the app and gives it its own token, using this computer’s hub sign-in.',
        children: const [
          SRow(
            key: ValueKey('connect-agent-row-claude'),
            label: 'Claude Code',
            stack: true,
            control: SCopyLine(
              claudeCommand,
              textKey: ValueKey('connect-agent-claude'),
            ),
          ),
          SRow(
            key: ValueKey('connect-agent-row-codex'),
            label: 'Codex',
            stack: true,
            control: SCopyLine(
              codexCommand,
              textKey: ValueKey('connect-agent-codex'),
            ),
          ),
        ],
      ),
    );
  }
}
