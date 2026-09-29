# Setting up Stripe

With Stripe set up, sharing live costs a subscription:

- A new account is **unpaid**.
- It can share live anyway for its first `FREE_TRIAL_DAYS` days (7 if unset).
- After that, `agent-graph watch-remote` opens the account page, and waits until the account subscribes, monthly or yearly.

Without Stripe set up (`STRIPE_MODE` unset, or any setting that mode needs unset), sharing live is free and every account is **active**. How it works is in `lib/billing.ts` and `lib/stripe.ts`.

`STRIPE_MODE` is `test` or `production`, and picks which keys and prices the site uses:

- `test` uses the `STRIPE_TEST_…` variables, from a Stripe **sandbox** (test mode). Nothing's charged.
- `production` uses the `STRIPE_…` ones, from Stripe's **live mode**.

Stripe keeps the two modes apart: products, prices, webhooks, the customer portal and keys all exist separately in each, and one mode's key can't use the other's prices. So do steps 1 to 6 twice: first in a sandbox, then in live mode. You can set both modes' variables at once, and switch between them with `STRIPE_MODE` alone.

## 1. The product and its two prices

1. Dashboard → **Product catalog** → **+ Create product**.
2. Name: `Agent Graph` (customers see it on Checkout, receipts and invoices).
3. Under pricing, choose **Recurring**, and enter the monthly price: amount and currency (say, 5.00 USD), **Billing period** Monthly. Leave "Usage-based" off: it's a flat price.
4. **Add product**.
5. Open the product, and in **Pricing**, **+ Add another price**: **Recurring**, the same currency, the yearly amount (say, 50.00 USD, cheaper than twelve months), **Billing period** Yearly. **Add price**.
6. Copy each price's ID (`price_…`) from its row's menu in **Pricing**:
   - the monthly one is `STRIPE_PRICE_MONTHLY` (`STRIPE_TEST_PRICE_MONTHLY` in a sandbox);
   - the yearly one is `STRIPE_PRICE_YEARLY` (`STRIPE_TEST_PRICE_YEARLY` in a sandbox).

`STRIPE_MONTHLY_LABEL` and `STRIPE_YEARLY_LABEL` are how the account page shows the costs, on its Subscribe buttons and beside them: write them to match the prices, say `$5 a month` and `$50 a year`. You could say what the yearly one saves: `$50 a year (2 months free)`. Stripe doesn't set them, so change them whenever you change the prices. They're the same in both modes, so keep the sandbox's prices the same as live.

Keep both prices on the one product: the customer portal (step 3) lets subscribers switch between prices of a product.

To change a price later, add a new price to the product, and point its variable at it. Existing subscribers keep the price they signed up at, until you move them in the Dashboard. (One on a price that's no longer either variable's is still subscribed; the account page just doesn't name its plan.)

## 2. Payment methods, including Link

Checkout is given no list of payment methods, so it offers every method you turn on that works for subscriptions.

1. Dashboard → **Settings** (the gear) → **Payments** → **Payment methods**.
2. In the default configuration, turn on:
   - **Cards**: on by default.
   - **Link**: under Wallets. It saves the card and the email for one-click payments anywhere Link is accepted.
   - **Apple Pay** and **Google Pay**: under Wallets. Stripe's hosted Checkout page needs no domain verification for them.
   - Anything else you want (SEPA Direct Debit, say). Checkout hides methods that can't pay a subscription.
3. Save.

## 3. The customer portal

The account page's **Manage billing** button opens Stripe's customer portal, where subscribers change their card, see invoices, or cancel.

1. Dashboard → **Settings** → **Billing** → **Customer portal**.
2. Turn on:
   - **Payment methods**: customers can update them.
   - **Invoice history**.
   - **Cancel subscriptions**, set to **At the end of the billing period**. The account page then says when it ends, and it stays active until then.
   - **Switch plans** (under Subscriptions → Customers can switch plans): add the **Agent Graph** product, with both prices, so subscribers can move between monthly and yearly. Choose how to prorate: **Prorate charges and credits** is Stripe's default, and fair to both sides.
3. Under **Business information**, add a headline, and links to your terms and privacy policy if you have them.
4. **Save**. The portal won't open until it's been saved once in each mode.

## 4. The API key

Dashboard → **Developers** → **API keys**.

Either use the **Secret key** (`sk_live_…` live, `sk_test_…` in a sandbox), or, better, **+ Create restricted key** with only what the site uses:

| Resource          | Permission |
| ----------------- | ---------- |
| Customers         | Write      |
| Checkout Sessions | Write      |
| Subscriptions     | Read       |
| Customer portal   | Write      |

Everything else can stay at None. The live key is `STRIPE_SECRET_KEY`, and the sandbox's is `STRIPE_TEST_SECRET_KEY`.

## 5. The webhook

Stripe tells the site when a subscription starts, renews, fails to be paid or ends.

1. Dashboard → **Developers** → **Webhooks** → **+ Add destination** (or **Add endpoint**).
2. Events from **Your account**. Any API version will do: the site fetches each subscription again with its own client's version, rather than reading the event's copy.
3. Select these events:
   - `checkout.session.completed`
   - `customer.subscription.created`
   - `customer.subscription.updated`
   - `customer.subscription.deleted`
   - `customer.subscription.paused`
   - `customer.subscription.resumed`
4. Destination type **Webhook endpoint**, and the URL for the mode:
   - live mode: `https://agentgraph.chofter.com/api/stripe/webhook`
   - a sandbox: `https://agentgraph.chofter.com/api/stripe/webhook-test`
5. Create it, open it, and reveal the **Signing secret** (`whsec_…`). The live one is `STRIPE_WEBHOOK_SECRET`, and the sandbox's is `STRIPE_TEST_WEBHOOK_SECRET`.

Each endpoint takes only its own mode's events:

- `/api/stripe/webhook` checks signatures with `STRIPE_WEBHOOK_SECRET`, and refuses test mode events.
- `/api/stripe/webhook-test` checks them with `STRIPE_TEST_WEBHOOK_SECRET`, and refuses live ones.
- Each records what it's sent only while `STRIPE_MODE` is its mode. So a sandbox subscription never makes an account active in production.
- Otherwise it replies 200, so Stripe doesn't send the event again, and ignores it. Its reply says `"recorded": false`.

So both webhooks can stay set up, whichever mode the site's in, as long as both modes' secret keys and signing secrets are set. An endpoint whose mode's aren't set replies 404, and Stripe retries it for a few days.

## 6. Subscription settings

These are optional, in Dashboard → **Settings** → **Billing** → **Subscriptions and emails**:

- **Manage failed payments**: keep Smart Retries on. Choose what happens once they're all used up: **cancel the subscription** is simplest.
  - The account is unpaid as soon as Stripe marks the subscription anything but `active` or `trialing` (`past_due`, `unpaid`, `canceled`…).
  - A share already running carries on until the paid period's end, plus 3 days' grace for the renewal. It stops sooner if it's restarted, which checks the account again.
- **Emails**: turn on emails for failed payments, and for expiring cards, so subscribers can fix them before sharing stops.

## 7. The environment variables

In Vercel → the project → **Settings** → **Environment Variables**:

| Variable                     | Value                                                         |
| ---------------------------- | ------------------------------------------------------------- |
| `STRIPE_MODE`                | `production` to charge for real, or `test` to use the sandbox |
| `STRIPE_SECRET_KEY`          | live, step 4 (`sk_live_…` or `rk_live_…`)                     |
| `STRIPE_WEBHOOK_SECRET`      | live, step 5 (`whsec_…`)                                      |
| `STRIPE_PRICE_MONTHLY`       | live, step 1 (`price_…`)                                      |
| `STRIPE_PRICE_YEARLY`        | live, step 1 (`price_…`)                                      |
| `STRIPE_TEST_SECRET_KEY`     | sandbox, step 4 (`sk_test_…` or `rk_test_…`)                  |
| `STRIPE_TEST_WEBHOOK_SECRET` | sandbox, step 5 (`whsec_…`)                                   |
| `STRIPE_TEST_PRICE_MONTHLY`  | sandbox, step 1 (`price_…`)                                   |
| `STRIPE_TEST_PRICE_YEARLY`   | sandbox, step 1 (`price_…`)                                   |
| `STRIPE_MONTHLY_LABEL`       | the monthly price, as people should read it: `$5 a month`     |
| `STRIPE_YEARLY_LABEL`        | the yearly price, as people should read it: `$50 a year`      |
| `FREE_TRIAL_DAYS`            | optional: days free before subscribing; 7 if it's unset       |

Only the current mode's keys and prices have to be set. If `STRIPE_MODE` is set but something it needs isn't, the site stays free, and says what's missing once in its logs: `Stripe isn't set up, so sharing live is free: …`.

Then redeploy: the variables are read when requests come in, but Vercel applies new ones only to new deployments.

A simple arrangement:

- **Production** environment: `STRIPE_MODE=production`, with the live values.
- **Preview** environment: `STRIPE_MODE=test`, with the sandbox values. Point the sandbox's webhook at a Preview deployment's `/api/stripe/webhook-test`, and set `NEXT_PUBLIC_SITE_URL` to that address under Preview too: Stripe sends the browser back to it after Checkout and the portal.

Or try test mode on the production site itself: set `STRIPE_MODE=test` there for a while. The sandbox's webhook already points at it, at `/api/stripe/webhook-test`.

In test mode the account page says so, and gives the test card to pay with.

## 8. Try it in the sandbox

With `STRIPE_MODE=test` and the sandbox values set (on a Preview deployment, or locally in `.env.local`):

1. **Locally**, forward Stripe's events to your machine with the [Stripe CLI](https://docs.stripe.com/stripe-cli):
   ```bash
   stripe listen --forward-to localhost:3000/api/stripe/webhook-test
   ```
   It prints a `whsec_…` for this session: use that as `STRIPE_TEST_WEBHOOK_SECRET` locally.
2. Log in on the site, and open **Account**. It should say test mode, that sharing live is free until a week from now, and then both prices.
3. **Subscribe yearly** (or monthly). On Stripe's page, check the price is the right one, and pay with the card `4242 4242 4242 4242` (any future date, any CVC), or with Link.
   - With more than two days of free time left, Stripe starts a trial that ends when the free days do. The card is saved, but nothing's charged until then. The account page says when the first payment is.
   - Stripe sends you back to the account page, which should say you're subscribed, and to which plan.
4. **Manage billing** → switch to the other plan. Back on the account page, it should name the new plan.
5. **Manage billing** → cancel. Back on the account page, it should say when the subscription ends.
6. To see sharing stop at the end of the free days, set `FREE_TRIAL_DAYS=0` and redeploy. Every unpaid account then has to subscribe first. Run `agent-graph watch-remote --url=<the deployment>`: it should open the account page, and start sharing within 5 seconds of you subscribing.
7. To see a subscription end, find it in Dashboard → **Billing** → **Subscriptions**, and **Cancel subscription** → **Immediately**. The webhook marks the account unpaid, and the account page asks it to subscribe again.

## 9. Go live

1. Switch the Dashboard to live mode, and do steps 1 to 6 again there.
2. Activate the account for live payments, if Stripe hasn't yet: Dashboard → **Settings** → **Business**, for your business details and bank account.
3. Set the live values in Vercel (step 7), and `STRIPE_MODE=production` for Production. Redeploy.

Subscriptions made in test mode don't carry over: an account that subscribed in the sandbox is unpaid in production until it subscribes there. A test subscription recorded on an account names a sandbox price, so the account page doesn't name its plan.

## Accounts made before this

An account made before subscriptions has no status, so it's treated as unpaid. Its free days count from when it was made, so for most they're already over, and its next `watch-remote` asks it to subscribe.

To let an account keep sharing without paying, set its document's `status` to `active` in Firestore (`users/{uid}`), and leave `subscription` unset: an active account with no subscription never runs out.
