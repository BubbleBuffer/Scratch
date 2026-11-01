// The merge proposals Triage made: each entry names the page it keeps (the
// older) and the page it merges in (the newer), the reason, and Merge /
// Dismiss. A merged entry says "Merged" and offers Undo. A full-screen page
// on a phone, a centred panel on desktop. The list is the store's; this file
// holds only the widget, so it can move into a Review view later.

import 'package:flutter/widgets.dart';

import '../../shell/navigation.dart' show openPage;
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'triage_copy.dart';
import 'triage_store.dart';

/// Shows the proposals as a screen (phone) or a panel (desktop).
Future<void> showProposals(BuildContext context) {
  final store = TriageStore.of(context.app);
  final opener = context;
  Widget panel(BuildContext ctx) => ProposalsPanel(
    store: store,
    onOpenPage: (id, title) {
      Navigator.of(ctx).pop();
      if (opener.mounted) openPage(opener, id, title);
    },
  );
  if (context.mobile) {
    return pushScreen<void>(
      context,
      (ctx) => ScreenScaffold(title: proposalsTitle, body: panel(ctx)),
      name: '/triage-proposals',
    );
  }
  return showModalPanel<void>(
    context,
    (ctx) => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const _PanelHeader(),
        Expanded(child: panel(ctx)),
      ],
    ),
    width: 560,
    height: 600,
  );
}

class _PanelHeader extends StatelessWidget {
  const _PanelHeader();
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      height: 48,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: c.borderSubtle)),
      ),
      child: Text(
        proposalsTitle,
        style: ts(context.type.ui, c.textPrimary, weight: 600),
      ),
    );
  }
}

class ProposalsPanel extends StatefulWidget {
  const ProposalsPanel({super.key, required this.store, this.onOpenPage});

  final TriageStore store;

  /// Called with a page's id and title when its title is tapped.
  final void Function(String id, String title)? onOpenPage;

  @override
  State<ProposalsPanel> createState() => _ProposalsPanelState();
}

class _ProposalsPanelState extends State<ProposalsPanel> {
  final _busy = <String>{};

  @override
  void initState() {
    super.initState();
    widget.store.addListener(_changed);
    widget.store.load();
  }

  @override
  void dispose() {
    widget.store.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  Future<void> _act(
    Object target,
    String label,
    Future<void> Function(String id) call,
  ) async {
    final id = target is TriageProposalDto ? target.id : target as String;
    if (!_busy.add(id)) return;
    setState(() {});
    try {
      await call(id);
    } catch (e) {
      if (mounted) showErrorToast(context, decisionFailed(label, e));
    } finally {
      _busy.remove(id);
      if (mounted) setState(() {});
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final items = widget.store.items;
    final applied = widget.store.applied;
    if (items.isEmpty && applied.isEmpty) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: widget.store.loaded
              ? Text(
                  proposalsEmpty,
                  key: const ValueKey('proposals-empty'),
                  textAlign: TextAlign.center,
                  style: ts(ty.ui, c.textSecondary, weight: 400),
                )
              : const SizedBox.shrink(),
        ),
      );
    }
    return ListView.separated(
      key: const ValueKey('proposals-list'),
      padding: EdgeInsets.symmetric(
        horizontal: context.mobile ? Layout.gutter : 12,
        vertical: 8,
      ),
      itemCount: items.length + applied.length,
      separatorBuilder: (_, _) => const SizedBox(height: 8),
      itemBuilder: (_, i) => i < items.length
          ? _entry(context, items[i])
          : _run(context, applied[i - items.length]),
    );
  }

  Widget _entry(BuildContext context, TriageProposalDto p) {
    final c = context.colors;
    final ty = context.type;
    final merged = p.status == TriageStatus.accepted;
    final busy = _busy.contains(p.id);
    final keep = displayTitle(p.keepTitle);
    final dropped = displayTitle(p.droppedTitle);
    final small = ts(ty.uiSmall, c.textTertiary, weight: 400);
    final h = context.mobile ? 42.0 : 32.0;
    return Container(
      key: ValueKey('proposal-${p.id}'),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: Radii.ctl,
        border: Border.all(color: c.borderSubtle),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _titleLine(context, 'Keeps', p.keepId, keep, 'proposal-keep-${p.id}'),
          const SizedBox(height: 4),
          _titleLine(
            context,
            'Merges in',
            p.droppedId,
            dropped,
            'proposal-drop-${p.id}',
          ),
          const SizedBox(height: 8),
          Text(
            p.reason,
            key: ValueKey('proposal-reason-${p.id}'),
            style: ts(ty.ui, c.textPrimary, weight: 400),
          ),
          const SizedBox(height: 6),
          Text(
            merged ? mergedNote(dropped) : mergeConsequence(keep, dropped),
            style: small,
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              if (merged) ...[
                Container(
                  key: ValueKey('proposal-merged-${p.id}'),
                  padding: const EdgeInsets.symmetric(
                    horizontal: 8,
                    vertical: 2,
                  ),
                  decoration: BoxDecoration(
                    color: c.okBg,
                    borderRadius: Radii.pill,
                  ),
                  child: Text(
                    'Merged',
                    style: ts(ty.uiSmall, c.textPrimary, weight: 600),
                  ),
                ),
                const Spacer(),
                SecondaryButton(
                  key: ValueKey('proposal-undo-${p.id}'),
                  label: 'Undo',
                  height: h,
                  onTap: busy
                      ? null
                      : () => _act(p, 'Undo', widget.store.undoMerge),
                ),
              ] else ...[
                const Spacer(),
                SecondaryButton(
                  key: ValueKey('proposal-dismiss-${p.id}'),
                  label: 'Dismiss',
                  height: h,
                  onTap: busy
                      ? null
                      : () => _act(p, 'Dismiss', widget.store.dismiss),
                ),
                const SizedBox(width: 8),
                PrimaryButton(
                  key: ValueKey('proposal-merge-${p.id}'),
                  label: 'Merge',
                  height: h,
                  onTap: busy ? () {} : () => _act(p, 'Merge', widget.store.merge),
                ),
              ],
            ],
          ),
        ],
      ),
    );
  }

  Widget _run(BuildContext context, TriageRunDto run) {
    final c = context.colors;
    final ty = context.type;
    final busy = _busy.contains(run.task);
    final h = context.mobile ? 42.0 : 32.0;
    return Container(
      key: ValueKey('applied-${run.task}'),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: Radii.ctl,
        border: Border.all(color: c.borderSubtle),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            appliedTitle,
            style: ts(ty.ui, c.textPrimary, weight: 600),
          ),
          const SizedBox(height: 6),
          for (final a in run.actions)
            Padding(
              padding: const EdgeInsets.only(bottom: 4),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: widget.onOpenPage == null
                    ? null
                    : () => widget.onOpenPage!(
                        a.pageId,
                        displayTitle(a.pageTitle),
                      ),
                child: Text(
                  a.kind == 'link'
                      ? appliedLink(
                          displayTitle(a.pageTitle),
                          displayTitle(a.targetTitle),
                        )
                      : appliedTag(displayTitle(a.pageTitle), a.target),
                  style: ts(ty.ui, c.accentText, weight: 400),
                ),
              ),
            ),
          const SizedBox(height: 2),
          Text(
            appliedUndoNote,
            style: ts(ty.uiSmall, c.textTertiary, weight: 400),
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              const Spacer(),
              SecondaryButton(
                key: ValueKey('applied-undo-${run.task}'),
                label: 'Undo',
                height: h,
                onTap: busy
                    ? null
                    : () => _act(
                        run.task,
                        'Undo',
                        (_) => widget.store.undoRun(run.task),
                      ),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _titleLine(
    BuildContext context,
    String role,
    String id,
    String title,
    String key,
  ) {
    final c = context.colors;
    final ty = context.type;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.baseline,
      textBaseline: TextBaseline.alphabetic,
      children: [
        SizedBox(
          width: 76,
          child: Text(role, style: ts(ty.uiSmall, c.textTertiary, weight: 400)),
        ),
        Expanded(
          child: GestureDetector(
            key: ValueKey(key),
            behavior: HitTestBehavior.opaque,
            onTap: widget.onOpenPage == null
                ? null
                : () => widget.onOpenPage!(id, title),
            child: Text(
              title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(ty.ui, c.accentText, weight: 600),
            ),
          ),
        ),
      ],
    );
  }
}
