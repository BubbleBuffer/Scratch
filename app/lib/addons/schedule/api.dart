// The schedule add-on's public API. The app imports only this file.
// A schedule is a note type of its own: its definition and status live in the
// page's type data (format.dart). The engine (engine.dart) answers for this
// device's own runner (through the vault) and for hub runners; another desktop
// is shown from the vault and never called from here.

import 'package:flutter/widgets.dart';

import '../../registry/addon_registry.dart';
import '../../registry/note_type_registry.dart';
import '../../src/rust/api/clock.dart' show deviceTimeZone;
import '../../ui/note_preview.dart';
import 'backend.dart';
import 'block.dart';
import 'create.dart';
import 'embed.dart';
import 'engine.dart';
import 'format.dart';
import 'local_runner.dart';
import 'settings.dart';
import 'storage.dart';
import 'view.dart';

export 'create.dart' show createSchedule, createWeeklySchedule;
export 'embed.dart' show ScheduleEmbedCard;
export 'view.dart' show SchedulePage;

String? _zone;

/// The device's IANA zone for new schedules. Falls back to UTC when the
/// device has no zone to report.
String _deviceZone() {
  if (_zone != null) return _zone!;
  try {
    final zone = deviceTimeZone().trim();
    _zone = zone.isEmpty ? 'UTC' : zone;
  } catch (_) {
    _zone = 'UTC';
  }
  return _zone!;
}

class ScheduleAddon extends Addon {
  const ScheduleAddon();

  @override
  String get id => 'schedule';

  @override
  List<String> get dependsOn => const [];

  @override
  void register(AddonContext ctx) {
    ctx.registerNoteType(
      NoteType(
        id: scheduleType,
        label: 'Schedule',
        chip: noteChip,
        card: noteCard,
        usesEditor: false,
        embed: (context, args) => _scoped(
          context,
          ctx,
          ScheduleEmbedCard(
            key: ValueKey('inline-schedule-${args.pageId}'),
            pageId: args.pageId,
            title: args.title,
            deviceTimeZone: _deviceZone(),
            localDeviceId: ctx.services.vault(context).deviceId,
          ),
        ),
        fullPage: (context, args) => _scoped(
          context,
          ctx,
          SchedulePage(
            key: ValueKey('schedule-page-${args.pageId}'),
            pageId: args.pageId,
            title: args.title,
            titleController: args.titleController,
            onTitleChanged: args.onTitleChanged,
            deviceTimeZone: _deviceZone(),
            localDeviceId: ctx.services.vault(context).deviceId,
          ),
        ),
      ),
    );

    ctx.blockKinds.register(
      scheduleInsertBlock(
        kind: scheduleInsertKind,
        label: 'Schedule',
        hint: 'Make a note from a template on a schedule.',
        aliases: 'schedule recurring repeat automation',
        createTarget: (context, pageId) async => createSchedule(
          ctx.services.vault(context),
          parent: pageId,
          deviceTimeZone: _deviceZone(),
        ),
      ),
    );
    ctx.blockKinds.register(
      scheduleInsertBlock(
        kind: scheduleWeeklyInsertKind,
        label: 'Weekly note schedule',
        hint: 'Make a weekly note every Monday from a template.',
        aliases: 'weekly note schedule monday template journal',
        createTarget: (context, pageId) async {
          final vault = ctx.services.vault(context);
          final backend = _backendFor(context, ctx);
          return createWeeklySchedule(
            vault,
            parent: pageId,
            nodes: await _listNodes(backend),
            deviceTimeZone: _deviceZone(),
          );
        },
      ),
    );

    // The runner setting exists on desktop only. The Add-ons panel does not
    // honour a section's isVisible, so Android never registers it.
    if (localRunnerPlatform) {
      ctx.settingsSections.register(scheduleRunnerSetting(ctx.services));
    }

    ctx.slashItems.register(
      SlashItem(
        id: scheduleInsertKind,
        label: 'Schedule',
        description: 'Make a note from a template on a schedule.',
        blockKind: scheduleInsertKind,
        run: (_) {},
      ),
    );
    ctx.slashItems.register(
      SlashItem(
        id: scheduleWeeklyInsertKind,
        label: 'Weekly note schedule',
        description: 'A note every Monday, from the Weekly note template.',
        blockKind: scheduleWeeklyInsertKind,
        run: (_) {},
      ),
    );
  }
}

/// The engine backend for the vault on screen, shared by every schedule view
/// on it.
ScheduleBackend _backendFor(BuildContext context, AddonContext ctx) =>
    EngineScheduleBackend.forVault(
      ctx.services.vault(context),
      ctx.services.settings(context),
    );

/// Wraps [child] in the storage and backend the schedule widgets read.
Widget _scoped(BuildContext context, AddonContext ctx, Widget child) =>
    ScheduleScope(
      storage: VaultScheduleStorage(ctx.services.vault(context)),
      child: ScheduleBackendScope(
        backend: _backendFor(context, ctx),
        child: child,
      ),
    );

Future<List<ScheduleNode>> _listNodes(ScheduleBackend backend) async {
  try {
    return await backend.listNodes();
  } catch (_) {
    return const [];
  }
}
