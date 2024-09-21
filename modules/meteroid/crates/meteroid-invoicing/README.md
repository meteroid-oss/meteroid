# Invoicing

This crate is responsible for pdf/XML invoice generation from our core invoice domain,
and for its issuance to the customer and public authorities.

The PDF is saved in S3/object storage, and a link is returned to the caller.

## Pdf Engine Rationale

We went for a HTML-to-PDF approach based on chromium as it was the easiest solution for a first implementation, suitable
for OSS scale.

We rely on Gotenberg workers (just a go wrapper to manage a chromium instance) for this.
An instance can only handle a single task at a time, we need to scale instances horizontally.

A vastly more performant option would be to rely on binary generation instead of conversion, but existing solutions are really
low level (manual pixel positioning etc) so it'll be a pain to maintain.

In the next few months we may have suitable template-to-pdf options though :

- https://github.com/typst/typst/issues/4411
- https://github.com/DioxusLabs/blitz/issues/128

## Templating

We use Maud to generate html from rust DSL, plus TailwindCSS for quick styling.

We are currently NOT including tailwind in our build pipeline (that felt unnecessary), so make sure to run `pnpm css` to
build the css if you modify a template as part of the dev process.

## Scaling

We have a sync worker per Gotenberg instance.

We therefore need some kind of queue to manage the pdf generation requests.

We will handle it through an outbox pattern, with or without Kafka/Debezium in between.

## next steps (multiple crates)

- XML einvoicing (Factur-X/ZUGFeRD, UBL, CII, etc), starting with Factur-X & UBL 2.1
- PDF/A-3b attachments
- Email delivery
- Upload xml to public authority
- Preview pdf (just expose the html)
- tax reporting ?
