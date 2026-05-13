//
//  AppDelegate.swift
//  Davids Pizzeria — staff app
//
//  Lifecycle entry point. Handles:
//   - SceneDelegate registration (UIKit's standard launch flow)
//   - APNs device-token callbacks (the system hands us the token here
//     once `registerForRemoteNotifications` succeeds)
//   - Forwards the token + the cookie-derived role to the server's
//     /api/register_push_token endpoint
//
//  Anything else (UI, navigation, the WebView itself) lives in
//  SceneDelegate / WebViewController.
//

import UIKit
import UserNotifications

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        // Push permission prompt the FIRST time the app launches. iOS
        // remembers the answer; we don't re-ask. If the user said "Don't
        // Allow", later calls quietly no-op — that's fine.
        UNUserNotificationCenter.current().delegate = self
        UNUserNotificationCenter.current().requestAuthorization(
            options: [.alert, .badge, .sound]
        ) { granted, error in
            guard granted, error == nil else {
                NSLog("[push] permission denied or errored: \(String(describing: error))")
                return
            }
            DispatchQueue.main.async {
                application.registerForRemoteNotifications()
            }
        }
        return true
    }

    // MARK: - APNs token callbacks

    /// iOS calls this once the device token is ready (~1s after launch
    /// on a connected device, never on the simulator unless you have
    /// the new "remote-push to simulator" setup; we test on a real
    /// device only for push).
    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        let tokenHex = deviceToken.map { String(format: "%02x", $0) }.joined()
        NSLog("[push] APNs token: \(tokenHex.prefix(8))…")
        Task {
            await PushRegistration.shared.register(token: tokenHex)
        }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        NSLog("[push] register failed: \(error)")
    }

    // MARK: - SceneDelegate handoff

    func application(
        _ application: UIApplication,
        configurationForConnecting connectingSceneSession: UISceneSession,
        options: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let cfg = UISceneConfiguration(name: "Default", sessionRole: connectingSceneSession.role)
        cfg.delegateClass = SceneDelegate.self
        return cfg
    }
}

extension AppDelegate: UNUserNotificationCenterDelegate {
    /// Called when a push arrives while the app is in the foreground.
    /// We show it as a banner anyway (default iOS behaviour suppresses).
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .badge])
    }

    /// Called when the user taps a notification. If the payload carries
    /// a `deep_link` field (set server-side in apns.rs), route the
    /// WebView to it.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        if let link = response.notification.request.content.userInfo["deep_link"] as? String,
           let url = URL(string: link) {
            NotificationCenter.default.post(name: .deepLink, object: url)
        }
        completionHandler()
    }
}

extension Notification.Name {
    static let deepLink = Notification.Name("eu.trahe.rusterando.deepLink")
}
