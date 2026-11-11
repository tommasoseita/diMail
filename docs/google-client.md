# Creating your Google client

In italiano: [docs/it/google-client.md](it/google-client.md)

diMail talks to Gmail directly from your Mac, with no server in between. To allow that, Google asks for an "OAuth client": the identity of the app that requests access to your mail. You create the client yourself, in your own Google Cloud project, and diMail uses it only on your Mac. It takes about ten minutes, once.

If your company uses Google Workspace, one client is enough for everyone: see [For a company on Google Workspace](#for-a-company-on-google-workspace).

## Why you need your own client

Reading and sending mail needs Gmail's `gmail.modify` scope, which Google classes as "restricted". A client shared by every diMail user would have to pass Google's verification and a security assessment, repeated every year. diMail is an open-source app with no server of its own, so each user, or each company, brings their own client instead.

It also means that nobody else is in the middle: the client, the consent and the tokens are yours.

## Before you start: Internal or External

The Google Cloud console is shown here in English. If yours is in another language, the items are in the same places.

| Your address | Audience of the client | What it means |
|---|---|---|
| Google Workspace (for example `name@company.com`) | **Internal** | Only accounts of your organization can sign in. No verification by Google, no warning at sign-in. You need to be a Workspace administrator, or to ask one. |
| Personal Gmail (`@gmail.com`) | **External**, status **In production** | At sign-in Google warns that the app isn't verified. That is expected: the app is yours. Left in "Testing" status, the access would expire every 7 days. |

## 1. The project

1. Open [console.cloud.google.com](https://console.cloud.google.com) with the account you will connect to diMail.
2. At the top, open the project picker and choose **New project**. Name: `diMail`. Create it and select it.

## 2. The Gmail API

1. Menu › **APIs & Services** › **Library**.
2. Search for **Gmail API**, open it and click **Enable**.

## 3. The consent screen

Menu › **APIs & Services** › **OAuth consent screen** (it opens the Google Auth Platform section) › **Get started**.

1. App information: name `diMail`, user support email (yours).
2. Audience: **Internal** for Google Workspace, **External** for personal Gmail (see the table above).
3. Contact information: your email. Accept the policy and click **Create**.
4. Optional but useful: in **Data Access** › **Add or remove scopes**, add `https://www.googleapis.com/auth/gmail.modify`. diMail asks only for this, plus your name, email and profile picture (`openid`, `email`, `profile`).
5. Only for **External**: in **Audience**, click **Publish app** and confirm. The status changes to "In production". You don't need to ask Google for verification to use the app yourself.

## 4. The client

1. In Google Auth Platform › **Clients** › **Create client**.
2. Application type: **Desktop app**. Name: `diMail`. Click **Create**.
3. In the window that opens, click **Download JSON**. Do it now: Google shows the client secret only at this moment. The file is named `client_secret_….apps.googleusercontent.com.json`.

## 5. In diMail

The app's labels are in Italian; the English meaning is in parentheses.

1. At the first launch diMail asks for the file: click **Scegli il file JSON** (Choose the JSON file) or drag the file onto the window.
2. Click **Accedi con Google** (Sign in with Google) and complete the sign-in in your browser.
   - With an External client, Google shows "Google hasn't verified this app": click **Advanced**, then **Go to diMail (unsafe)**. It is your app, on your account.
   - Leave the checkbox that lets diMail read and change your mail selected. Without it diMail can't work, and it tells you so.
3. The first sync starts.

The client stays in your Mac's Keychain, like the access token. To change it: **Impostazioni** (Settings, <kbd>⌘,</kbd>) › **Account Google** › **Sostituisci il JSON** (Replace the JSON). With a different client you have to sign in again; the mail already downloaded stays.

## Common problems

- **"Access blocked"**, saying that your administrator restricted access (Workspace): the organization blocks third-party apps. The administrator opens the Admin console › Security › Access and data control › API controls › Manage Third-Party App Access, looks the client up by its ID and marks it as **Trusted**.
- **"questo è il JSON di un client «Applicazione web»"** in diMail (this is the JSON of a "Web application" client): the client was created with the wrong type. Create another one of type **Desktop app**.
- **You have to sign in again every week:** the client is External and still in "Testing". Publish it (step 3.5).
- **"Gmail API has not been used in project … or it is disabled":** step 2 is missing.

More in [Troubleshooting](troubleshooting.md#signing-in-with-google).

## For a company on Google Workspace

With an **Internal** client, you do this once for the whole company:

1. A Workspace administrator, or someone who can create Google Cloud projects in the organization, follows steps 1 to 4, choosing **Internal** as the audience, and downloads the JSON. That is ONE client for everybody.
2. They put the file where only colleagues can see it, for example a Drive folder shared with the company.
3. Each colleague downloads diMail, chooses that file at the first launch and clicks **Accedi con Google** with their company account.

An Internal client needs no verification by Google and shows no warning at sign-in. It works only with the organization's accounts. Each colleague still signs in for themselves: the file gives access to nobody's mail.

## What about the client secret?

For a desktop app, Google doesn't treat the `client_secret` as a real secret: anyone who has the app could read it. What protects the access is the consent in the browser and PKCE, which diMail always uses. Don't publish the file anyway: it identifies your Google Cloud project.
