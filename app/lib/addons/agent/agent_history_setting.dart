// The chat setting "Chat history: Full | Compacted (trial)": a gear above the
// transcript opens a popover (a sheet on phones) with the two choices.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_store.dart';

const _description =
    'Compacted replaces older messages with chapter and episode summaries. The latest messages stay in full. Scroll up to read the summaries, or open a card for the original messages. Nothing is deleted.';

/// The gear that opens the chat settings.
class ChatSettingsButton extends StatelessWidget {
  const ChatSettingsButton({super.key, required this.store});
  final AgentStore store;

  Future<void> _open(BuildContext context) async {
    Widget panel(BuildContext _) => ChatSettingsPanel(store: store);
    if (context.mobile) {
      await showSheet<void>(
        context,
        (ctx) => SheetSurface(
          children: [const SheetHeader('Chat settings'), panel(ctx)],
        ),
      );
    } else {
      await showPopover<void>(
        context,
        globalRectOf(context),
        (ctx) => MenuSurface(width: 320, child: panel(ctx)),
      );
    }
  }

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: 'Chat settings',
    child: Builder(
      builder: (b) => Hover(
        key: const ValueKey('chat-settings'),
        onTap: () => _open(b),
        radius: Radii.md,
        child: SizedBox(
          width: 28,
          height: 28,
          child: Center(
            child: Icon20(
              Ico.settings,
              size: 16,
              color: context.colors.iconTertiary,
            ),
          ),
        ),
      ),
    ),
  );
}

/// The settings of one chat. Rebuilds when the store changes, so the choice
/// shows at once.
class ChatSettingsPanel extends StatelessWidget {
  const ChatSettingsPanel({super.key, required this.store});
  final AgentStore store;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final id = store.openId;
        final compacted = store.historyCompacted(id);
        void pick(bool value) {
          if (id != null && value != compacted) {
            store.setHistoryCompacted(id, value);
          }
        }

        return Padding(
          padding: const EdgeInsets.all(14),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                'Chat settings',
                style: ts(context.type.uiSmall, c.textPrimary, weight: 600),
              ),
              const SizedBox(height: 10),
              Row(
                children: [
                  Expanded(
                    child: Text(
                      'Chat history',
                      style: ts(context.type.uiSmall, c.textSecondary),
                    ),
                  ),
                  _Segments(compacted: compacted, onChanged: pick),
                ],
              ),
              const SizedBox(height: 10),
              Text(
                _description,
                style: ts(context.type.uiSmall, c.textTertiary),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _Segments extends StatelessWidget {
  const _Segments({required this.compacted, required this.onChanged});
  final bool compacted;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget segment(Key key, String label, String? trial, bool on, bool value) =>
        Semantics(
          inMutuallyExclusiveGroup: true,
          checked: on,
          button: true,
          child: Hover(
            key: key,
            onTap: () => onChanged(value),
            radius: Radii.sm,
            hoverColor: on ? kTransparent : null,
            background: on ? c.bgPrimary : null,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: SizedBox(
              height: 26,
              child: Center(
                child: Text.rich(
                  TextSpan(
                    text: label,
                    style: ts(
                      context.type.uiSmall,
                      on ? c.textPrimary : c.textSecondary,
                      weight: on ? 600 : 400,
                    ),
                    children: [
                      if (trial != null)
                        TextSpan(
                          text: ' $trial',
                          style: ts(context.type.uiSmall, c.textTertiary),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: BorderRadius.circular(Radii.md + 2),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          segment(
            const ValueKey('chat-history-full'),
            'Full',
            null,
            !compacted,
            false,
          ),
          segment(
            const ValueKey('chat-history-compacted'),
            'Compacted',
            '(trial)',
            compacted,
            true,
          ),
        ],
      ),
    );
  }
}
