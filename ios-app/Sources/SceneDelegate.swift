//
//  SceneDelegate.swift
//  Davids Pizzeria
//
//  Standard UIKit scene lifecycle. We just create one window, install
//  the WebViewController, and listen for Universal Link / deep-link
//  events.
//

import UIKit

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options connectionOptions: UIScene.ConnectionOptions
    ) {
        guard let windowScene = scene as? UIWindowScene else { return }

        let window = UIWindow(windowScene: windowScene)
        let root = WebViewController()
        window.rootViewController = root
        self.window = window
        window.makeKeyAndVisible()

        // Universal Link cold-start: if the app launched because the
        // user tapped a https://davidspizzeria.de/... URL, route the
        // WebView there directly instead of the home page.
        if let userActivity = connectionOptions.userActivities.first(where: {
            $0.activityType == NSUserActivityTypeBrowsingWeb
        }), let url = userActivity.webpageURL {
            root.loadURL(url)
        }

        // Push-tap deep link (the AppDelegate posts this when the user
        // taps a notification that carries a deep_link payload).
        NotificationCenter.default.addObserver(
            forName: .deepLink, object: nil, queue: .main
        ) { [weak root] note in
            if let url = note.object as? URL {
                root?.loadURL(url)
            }
        }
    }

    /// Universal Link warm-start (app already running, user taps a link
    /// elsewhere on the device). Same idea as the cold-start branch.
    func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
        guard userActivity.activityType == NSUserActivityTypeBrowsingWeb,
              let url = userActivity.webpageURL,
              let root = window?.rootViewController as? WebViewController else { return }
        root.loadURL(url)
    }
}
