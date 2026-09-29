"use client";

import {type FirebaseApp, getApps, initializeApp} from "firebase/app";
import {type Auth, connectAuthEmulator, getAuth} from "firebase/auth";

/**
 * Firebase Authentication in the browser, for signing in (app/login). Its
 * settings are public (they're in every page that signs in); what's secret
 * stays on the server. With NEXT_PUBLIC_FIREBASE_AUTH_EMULATOR_HOST set, it
 * signs in to the local emulator instead.
 */
export function clientAuth(): Auth {
  const app: FirebaseApp =
    getApps()[0] ??
    initializeApp({
      apiKey: process.env.NEXT_PUBLIC_FIREBASE_API_KEY,
      authDomain: process.env.NEXT_PUBLIC_FIREBASE_AUTH_DOMAIN,
      projectId: process.env.NEXT_PUBLIC_FIREBASE_PROJECT_ID,
      appId: process.env.NEXT_PUBLIC_FIREBASE_APP_ID,
    });
  const auth = getAuth(app);
  const emulator = process.env.NEXT_PUBLIC_FIREBASE_AUTH_EMULATOR_HOST;
  if (emulator && !auth.emulatorConfig) {
    connectAuthEmulator(auth, `http://${emulator}`, {disableWarnings: true});
  }
  return auth;
}
