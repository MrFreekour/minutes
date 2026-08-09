import type { Metadata } from "next";
import { Instrument_Serif } from "next/font/google";
import { GeistSans } from "geist/font/sans";
import { GeistMono } from "geist/font/mono";
import "./globals.css";

const instrumentSerif = Instrument_Serif({
  subsets: ["latin"],
  weight: "400",
  style: ["normal", "italic"],
  variable: "--font-instrument-serif",
  display: "swap",
});

// Title and description target the queries that actually drive impressions
// here ("minutes app", "minute app"), which were converting at 1.5% CTR from
// positions 4 to 7. Free and open source lead because they are the two claims
// no paid competitor in that SERP can make. Every negative below is literal:
// there is no signup, no API key is required, and the app is MIT licensed.
// Deliberately not "no cloud" — optional summarization can use a cloud LLM.
export const metadata: Metadata = {
  title: "Minutes: free, open-source meeting notes app",
  description:
    "Records meetings, calls, and voice memos, then transcribes them on your own machine. Searchable markdown you own. No account, no API keys, no subscription.",
  metadataBase: new URL("https://useminutes.app"),
  alternates: { canonical: "/" },
  icons: {
    icon: [
      { url: "/favicon.svg", type: "image/svg+xml" },
    ],
  },
  openGraph: {
    title: "Minutes: free, open-source meeting notes app",
    description:
      "Records meetings, calls, and voice memos, transcribed on your own machine. Searchable markdown you own. Free and open source.",
    type: "website",
    url: "https://useminutes.app",
    siteName: "minutes",
  },
  twitter: {
    card: "summary",
    title: "Minutes: free, open-source meeting notes app",
    description:
      "Meetings, calls, and voice memos transcribed on your own machine. Markdown you own. Free, MIT licensed.",
  },
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html
      lang="en"
      className={`${GeistSans.variable} ${GeistMono.variable} ${instrumentSerif.variable}`}
    >
      <head>
        <link rel="alternate" type="text/plain" href="/llms.txt" />
        <meta
          name="theme-color"
          media="(prefers-color-scheme: light)"
          content="#F8F4ED"
        />
        <meta
          name="theme-color"
          media="(prefers-color-scheme: dark)"
          content="#0D0D0B"
        />
      </head>
      <body className="font-sans antialiased">{children}</body>
    </html>
  );
}
