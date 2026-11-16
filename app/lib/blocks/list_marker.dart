// Shared layout of list items: a 24 px marker column before the text.

import 'package:flutter/widgets.dart';

import '../registry/block_registry.dart';
import '../ui/tokens.dart';
import '../ui/layout.dart';

Widget markerRow(BlockContext b, Widget marker, Widget text) => Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(width: 24, height: b.lineHeight, child: marker),
        const Gap(Space.hair),
        Expanded(child: text),
      ],
    );
