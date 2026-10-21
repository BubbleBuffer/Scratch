// The compact schedule card shown inside a note: what it does, when, where it
// runs and its state. Tapping it opens the full schedule page.

import 'package:flutter/widgets.dart';

import '../../services/legacy_host.dart' show openPage;
import '../../ui/database/widgets.dart' show Pill;
import '../../ui/format.dart' show displayTitle;
import '../../ui/tokens.dart';
import 'backend.dart';
import 'controller.dart';
import 'format.dart';
import 'storage.dart';
import 'view.dart' show runTimeLabel, scheduleHealthLabel;

class ScheduleEmbedCard extends StatefulWidget {
  const ScheduleEmbedCard({
    super.key,
    required this.pageId,
    this.title = '',
    this.deviceTimeZone = 'UTC',
    this.localDeviceId = '',
  });

  final String pageId;
  final String title;
  final String deviceTimeZone;
  final String localDeviceId;

  @override
  State<ScheduleEmbedCard> createState() => _ScheduleEmbedCardState();
}

class _ScheduleEmbedCardState extends State<ScheduleEmbedCard> {
  ScheduleController? _c;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_c != null) return;
    _c = ScheduleController(
      pageId: widget.pageId,
      storage: ScheduleScope.storageOf(context),
      backend: ScheduleBackendScope.of(context),
      deviceTimeZone: widget.deviceTimeZone,
      localDeviceId: widget.localDeviceId,
    )..load();
  }

  @override
  void dispose() {
    _c?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ctl = _c!;
    return ListenableBuilder(
      listenable: ctl,
      builder: (context, _) {
        final trigger = ctl.def.trigger;
        final runner = ctl.nodeOf(ctl.def.runner);
        final runs = ctl.nextRuns;
        final runnerText = ctl.def.runner == null
            ? 'No runner'
            : (runner?.name ?? 'Runner unavailable');
        return GestureDetector(
          key: ValueKey('schedule-embed-${widget.pageId}'),
          behavior: HitTestBehavior.opaque,
          onTap: () => openPage(context, widget.pageId, ''),
          child: Container(
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: c.bgSecondary,
              borderRadius: Radii.panel,
              border: Border.all(color: c.border),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        displayTitle(widget.title),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: ts(const TypeSpec(15, 20, 650), c.textPrimary),
                      ),
                    ),
                    const SizedBox(width: 8),
                    Pill(
                      scheduleHealthLabel(ctl.health),
                      status: true,
                      small: true,
                    ),
                  ],
                ),
                const SizedBox(height: 4),
                Text(
                  '${triggerInWords(trigger)} · $runnerText',
                  key: const ValueKey('schedule-embed-words'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(const TypeSpec(13, 18, 450), c.textSecondary),
                ),
                const SizedBox(height: 2),
                Text(
                  runs.isEmpty
                      ? 'No next run'
                      : 'Next: ${runTimeLabel(runs.first)}',
                  style: ts(const TypeSpec(13, 18, 450), c.textSecondary),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}
