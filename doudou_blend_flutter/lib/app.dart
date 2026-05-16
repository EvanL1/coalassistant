import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import 'ui/screens/contracts_screen.dart';
import 'ui/screens/customers_screen.dart';
import 'ui/screens/me_screen.dart';
import 'ui/screens/quotes_screen.dart';
import 'ui/screens/today_screen.dart';
import 'ui/shell.dart';
import 'ui/theme.dart';

class DoudouBlendApp extends StatelessWidget {
  DoudouBlendApp({super.key});

  final _router = GoRouter(
    initialLocation: '/today',
    routes: [
      StatefulShellRoute.indexedStack(
        builder: (context, state, shell) => AppShell(navigationShell: shell),
        branches: [
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/today',
                builder: (_, __) => const TodayScreen()),
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/customers',
                builder: (_, __) => const CustomersScreen()),
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/quotes',
                builder: (_, __) => const QuotesScreen()),
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/contracts',
                builder: (_, __) => const ContractsScreen()),
          ]),
          StatefulShellBranch(routes: [
            GoRoute(
                path: '/me',
                builder: (_, __) => const MeScreen()),
          ]),
        ],
      ),
    ],
  );

  @override
  Widget build(BuildContext context) {
    return MaterialApp.router(
      title: '豆哥配煤',
      debugShowCheckedModeBanner: false,
      theme: buildAppTheme(),
      routerConfig: _router,
    );
  }
}
