import { type AppOptions, applicationDefault, cert, getApps, initializeApp } from "firebase-admin/app";
import { type Firestore, getFirestore } from "firebase-admin/firestore";

/**
 * The Firestore client, created once per server instance.
 *
 * - With FIRESTORE_EMULATOR_HOST set, it talks to the local emulator.
 * - With FIREBASE_SERVICE_ACCOUNT (a service account's JSON key), it uses that.
 * - Otherwise it uses Google's application default credentials.
 */
// Kept on globalThis: Next bundles pages and API routes separately, and they
// must share one client (Firestore can only be configured once per app).
const shared = globalThis as { agentGraphFirestore?: Firestore };

export function firestore(): Firestore {
  if (!shared.agentGraphFirestore) {
    const app = getApps()[0] ?? initializeApp(options());
    shared.agentGraphFirestore = getFirestore(app);
  }
  return shared.agentGraphFirestore;
}

function options(): AppOptions {
  if (process.env.FIRESTORE_EMULATOR_HOST) {
    return { projectId: process.env.FIREBASE_PROJECT_ID || "demo-agent-graph" };
  }
  const key = process.env.FIREBASE_SERVICE_ACCOUNT;
  if (key) {
    const account = JSON.parse(key);
    return { credential: cert(account), projectId: account.project_id };
  }
  return { credential: applicationDefault(), projectId: process.env.FIREBASE_PROJECT_ID };
}
