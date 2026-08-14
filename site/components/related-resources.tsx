import { SectionLabel } from "@/components/section-label";
import { RELATED } from "@/lib/related";

/** "Keep reading" block at the foot of a resource page.
 *
 * Renders nothing when a page has no curated relations, so adding a page
 * without an entry in `RELATED` degrades to the previous behaviour rather than
 * rendering an empty section.
 */
export function RelatedResources({ slug }: { slug: string }) {
  const links = RELATED[slug];
  if (!links || links.length === 0) return null;

  return (
    <section className="mt-14">
      <SectionLabel label="Keep reading" />
      <ul className="space-y-4">
        {links.map((link) => (
          <li key={link.href}>
            <a
              href={link.href}
              className="text-[15px] leading-7 text-[var(--accent)] hover:underline"
            >
              {link.label}
            </a>
            <p className="mt-1 text-[14px] leading-6 text-[var(--text-secondary)]">
              {link.note}
            </p>
          </li>
        ))}
      </ul>
    </section>
  );
}
