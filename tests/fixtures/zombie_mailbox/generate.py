import json, os, sys
out = sys.argv[1]
ME = "Alex Example <alex@example.org>"
threads = []  # (key, subject, expect, note, [msgs])
# msg: (folder, days_ago, from, to, body, extra_headers)
def T(key, subject, expect, note, msgs): threads.append((key, subject, expect, note, msgs))
T("claim", "Claim No. 4471-B status", "awaiting_reply", "asked the insurer, never heard back",
  [("sent", 30, ME, "Claims Desk <claims@insurer.example>", "Hello,\n\nCould you tell me the status of claim No. 4471-B? I sent all documents last month.\n\nThanks,\nAlex", {})])
T("quote", "Kitchen renovation quote", "promise_lapsed", "contractor promised a quote by Friday",
  [("sent", 40, ME, "Bob Builder <bob@builders.example>", "Hi Bob,\n\nCould you send me a quote for the kitchen?\n\nAlex", {}),
   ("inbox", 28, "Bob Builder <bob@builders.example>", ME, "Hi Alex,\n\nThanks, I'll get back to you with a quote by Friday.\n\nBob", {})])
T("deposit", "Deposit return flat 3", "you_owe_reply", "landlord asked for the new address",
  [("sent", 25, ME, "Lena Landlord <lena@flats.example>", "Hi Lena,\n\nI moved out on the 1st. When will the deposit come back?\n\nAlex", {}),
   ("inbox", 20, "Lena Landlord <lena@flats.example>", ME, "Hi Alex,\n\nCould you send me your new address so I can return the deposit?\n\nLena", {})])
T("contract", "Signed contract for the accountant", "you_promised", "I promised the signed contract and never sent it",
  [("inbox", 19, "Anna Accounts <anna@acct.example>", ME, "Hi Alex,\n\nDo you have the signed contract for me?\n\nAnna", {}),
   ("sent", 18, ME, "Anna Accounts <anna@acct.example>", "Hi Anna,\n\nI'll send the signed contract tomorrow.\n\nAlex", {})])
T("visa", "Visa support letter request", "promise_lapsed", "only an auto-acknowledgement came",
  [("sent", 35, ME, "Visa Office <visa@consulate.example>", "Hello,\n\nPlease issue the support letter for my visa application, reference VX-2210.\n\nAlex", {}),
   ("inbox", 35, "Visa Office <visa@consulate.example>", ME, "We received your request and will respond within 3 business days.", {"Auto-Submitted": "auto-replied"})])
T("venue", "Venue booking for the 12th", None, "answered and thanked",
  [("sent", 30, ME, "Venue <events@venue.example>", "Hi,\n\nIs the hall still free on the 12th?\n\nAlex", {}),
   ("inbox", 29, "Venue <events@venue.example>", ME, "Hi Alex,\n\nYes, it's confirmed for the 12th.\n\nVenue team", {}),
   ("sent", 29, ME, "Venue <events@venue.example>", "Thanks!", {})])
T("invoice", "Invoice 2026-118 payment", None, "they confirmed payment, nothing open",
  [("sent", 26, ME, "Billing <billing@supplier.example>", "Hello,\n\nCan you confirm you received payment for invoice 2026-118?\n\nAlex", {}),
   ("inbox", 24, "Billing <billing@supplier.example>", ME, "Hello Alex,\n\nPayment received. The invoice is paid in full.\n\nBilling", {})])
T("deals", "Our biggest autumn deals", None, "newsletter",
  [("inbox", 21, "Shop News <news@shop.example>", ME, "Have you seen our new deals? Please visit the shop today!", {"List-Unsubscribe": "<mailto:unsub@shop.example>", "List-Id": "<news.shop.example>"})])
T("digest", "Your weekly digest", None, "no-reply digest",
  [("inbox", 30, "Digest <noreply@social.example>", ME, "Could you be missing out? See what your friends posted this week.", {})])
T("receipt", "Your order 88213 receipt", None, "bulk receipt",
  [("inbox", 33, "Orders <orders@shop.example>", ME, "Thanks for your order. Please keep this receipt.", {"Precedence": "bulk"})])
T("thanks", "Wrapping up the workshop", None, "closed exchange plus out-of-office",
  [("sent", 20, ME, "Kim Trainer <kim@training.example>", "Thanks for everything, all done.", {}),
   ("inbox", 20, "Kim Trainer <kim@training.example>", ME, "I am out of office until Monday. I will reply when I return.", {"Auto-Submitted": "auto-replied", "X-Autoreply": "yes"})])
T("recent", "Quick question about Saturday", None, "too recent to be stale",
  [("sent", 3, ME, "Sam Friend <sam@friends.example>", "Are we still on for Saturday?", {})])
T("pitch", "Grow your sales 10x", None, "cold sales pitch",
  [("inbox", 25, "Growth Guru <guru@leads.example>", ME, "Hi Alex, can we book a call this week?", {})])
T("ghnotif", "[kwant/kwant] CI failed on main", None, "GitHub notification email",
  [("inbox", 22, "GitHub <notifications@github.com>", ME, "Could not build: see the run for details.", {"List-Id": "<kwant.kwant.github.com>", "Precedence": "list"})])
T("bounce", "Undelivered Mail Returned to Sender", None, "bounce",
  [("inbox", 27, "Mail Delivery System <MAILER-DAEMON@mx.example.org>", ME, "Your message could not be delivered. Please check the address.", {"Auto-Submitted": "auto-generated"})])
T("noreply", "Question about my account", None, "written to a no-reply address",
  [("sent", 30, ME, "Bank <no-reply@bank.example>", "Can you tell me why my card was blocked?", {})])
T("job", "Application: Senior Rust Engineer", "awaiting_reply", "job application never answered",
  [("sent", 45, ME, "Hiring <jobs@company.example>", "Dear hiring team,\n\nPlease find my application attached. I look forward to hearing from you.\n\nAlex", {})])
T("refund", "Refund for order 7731", "promise_lapsed", "refund 'being reviewed' for weeks",
  [("sent", 30, ME, "Support <help@store.example>", "Hello,\n\nPlease refund order 7731, the item arrived broken.\n\nAlex", {}),
   ("inbox", 22, "Support <help@store.example>", ME, "Hello,\n\nYour refund request is being reviewed.\n\nSupport", {})])
T("rhetorical", "That concert last night", None, "a rhetorical question from a friend",
  [("sent", 31, ME, "Max Friend <max@friends.example>", "The concert was amazing.", {}),
   ("inbox", 30, "Max Friend <max@friends.example>", ME, "Haha, can you believe how loud it was?", {})])
T("pleasantry", "Slides from today's talk", None, "closing pleasantry, nothing expected",
  [("sent", 20, ME, "Org Team <talks@meetup.example>", "Here are the slides from today. Let me know if you have questions.", {})])
T("dentist", "Appointment confirmation", "you_owe_reply", "dentist asked me to confirm",
  [("sent", 18, ME, "Dental Clinic <desk@dental.example>", "Hello, I'd like an appointment for a check-up, please.", {}),
   ("inbox", 16, "Dental Clinic <desk@dental.example>", ME, "Hello Alex,\n\nPlease confirm your appointment for Tuesday at 10:00.\n\nDesk", {})])
T("docs", "Documents for the mortgage", None, "answered after my follow-up",
  [("sent", 30, ME, "Mira Broker <mira@mortgage.example>", "Hi Mira, could you send the documents list?", {}),
   ("sent", 20, ME, "Mira Broker <mira@mortgage.example>", "Following up on this: any update?", {}),
   ("inbox", 19, "Mira Broker <mira@mortgage.example>", ME, "Sorry for the delay, here are the documents. All set.", {})])
T("part", "Replacement part for the boiler", "promise_lapsed", "promised a part next week, nothing came",
  [("sent", 32, ME, "Heat Co <service@heat.example>", "Hello, the boiler part broke again. Can you replace it?", {}),
   ("inbox", 25, "Heat Co <service@heat.example>", ME, "Hello Alex,\n\nWe will send you the replacement part next week.\n\nHeat Co", {})])
T("lease", "Signed lease", "awaiting_reply", "awaiting a countersignature, worded without a question",
  [("sent", 30, ME, "Rita Realty <rita@realty.example>", "Hi Rita,\n\nSending over the signed lease; awaiting your countersignature.\n\nAlex", {})])
T("library", "Library card renewal", None, "answered with a statement",
  [("sent", 28, ME, "Library <desk@library.example>", "Could you renew my library card?", {}),
   ("inbox", 27, "Library <desk@library.example>", ME, "Done, your card is renewed until next year.", {})])

HOLD = len(threads)
T("passport", "Passport renewal appointment", "awaiting_reply", "holdout: asked for a slot, silence",
  [("sent", 21, ME, "Passport Office <appointments@gov.example>", "Hello,\n\nIs there an appointment available next month for a passport renewal?\n\nAlex", {})])
T("photos", "Wedding photos", "promise_lapsed", "holdout: photographer will send the album",
  [("sent", 40, ME, "Pia Photo <pia@photo.example>", "Hi Pia, any news on the album?", {}),
   ("inbox", 30, "Pia Photo <pia@photo.example>", ME, "Hi Alex! Editing is almost done, I'll send you the link soon.", {})])
T("review", "Draft chapter for review", "you_owe_reply", "holdout: editor asked for my comments",
  [("sent", 30, ME, "Eve Editor <eve@press.example>", "Here is chapter 3.", {}),
   ("inbox", 22, "Eve Editor <eve@press.example>", ME, "Thanks Alex. Would you send me your comments on my edits by Friday?", {})])
T("webinar", "Join our webinar on Thursday", None, "holdout: marketing with a question in it",
  [("inbox", 24, "Events <events@saas.example>", ME, "Want to learn more? Register now!", {"List-Unsubscribe": "<https://saas.example/u>"})])
T("lunch", "Lunch last week", None, "holdout: friendly closer",
  [("inbox", 26, "Ivo Pal <ivo@friends.example>", ME, "Great lunch, let's do it again!", {}),
   ("sent", 25, ME, "Ivo Pal <ivo@friends.example>", "Absolutely, see you soon.", {})])
T("ticket", "Support ticket #5521", None, "holdout: resolved ticket",
  [("sent", 30, ME, "Help Desk <support@isp.example>", "My internet keeps dropping. Can you check the line?", {}),
   ("inbox", 28, "Help Desk <support@isp.example>", ME, "We fixed the line fault. Ticket #5521 is now closed.", {})])

os.makedirs(os.path.join(out, "inbox"), exist_ok=True)
os.makedirs(os.path.join(out, "sent"), exist_ok=True)
manifest = []
n = 0
for idx, (key, subject, expect, note, msgs) in enumerate(threads):
    root = None
    prev = None
    refs = []
    for i, (folder, days, frm, to, body, extra) in enumerate(msgs):
        n += 1
        mid = f"<{key}-{i}@fixture.example>"
        subj = subject if i == 0 else "Re: " + subject
        h = [f"From: {frm}", f"To: {to}", f"Subject: {subj}", f"Date: {{{{DATE:{days}}}}}", f"Message-ID: {mid}"]
        if prev:
            h.append(f"In-Reply-To: {prev}")
            h.append("References: " + " ".join(refs))
        for k, v in extra.items():
            h.append(f"{k}: {v}")
        h += ["MIME-Version: 1.0", "Content-Type: text/plain; charset=utf-8"]
        text = "\r\n".join(h) + "\r\n\r\n" + body.replace("\n", "\r\n") + "\r\n"
        with open(os.path.join(out, folder, f"{key}-{i}.eml"), "w", newline="") as f:
            f.write(text)
        refs.append(mid)
        prev = mid
    manifest.append({"key": key, "subject": subject, "expect": expect, "note": note, "holdout": idx >= HOLD})
with open(os.path.join(out, "manifest.json"), "w") as f:
    json.dump({"me": ["alex@example.org"], "threads": manifest}, f, indent=1)
print(len(threads), "threads", n, "messages")
