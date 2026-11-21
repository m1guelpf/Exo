//! The invented people and companies in the fixture mailbox. None are real.

/// How someone relates to the account holder, which decides labels, categories and contacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Circle {
    /// The account holder.
    Me,
    /// Works at Brightmoor (in the directory).
    Colleague,
    /// Customers, partners, investors, vendors (saved contacts).
    Work,
    /// Friends and family (saved contacts).
    Personal,
    /// People the user has emailed but never saved ("other contacts").
    Acquaintance,
}

#[derive(Debug, Clone, Copy)]
pub struct Person {
    /// Short handle used by the conversation scripts.
    pub key: &'static str,
    pub first: &'static str,
    pub last: &'static str,
    pub email: &'static str,
    pub org: &'static str,
    pub title: &'static str,
    pub phone: &'static str,
    pub circle: Circle,
}

impl Person {
    pub fn name(&self) -> String {
        format!("{} {}", self.first, self.last)
    }

    pub fn mailbox(&self) -> gmail::mime::Mailbox {
        gmail::mime::Mailbox::new(self.name(), self.email)
    }

    pub fn is_me(&self) -> bool {
        self.circle == Circle::Me
    }
}

use Circle::*;

pub const DOMAIN: &str = "brightmoor.co";

#[rustfmt::skip]
pub static PEOPLE: &[Person] = &[
    Person { key: "maya", first: "Maya", last: "Okafor", email: "maya@brightmoor.co", org: "Brightmoor", title: "Head of Product", phone: "+1 917-555-0148", circle: Me },
    // Brightmoor
    Person { key: "daniel", first: "Daniel", last: "Reyes", email: "daniel@brightmoor.co", org: "Brightmoor", title: "CEO & Co-founder", phone: "+1 917-555-0101", circle: Colleague },
    Person { key: "priya", first: "Priya", last: "Raman", email: "priya@brightmoor.co", org: "Brightmoor", title: "Engineering Lead", phone: "+1 646-555-0177", circle: Colleague },
    Person { key: "tom", first: "Tom", last: "Becker", email: "tom@brightmoor.co", org: "Brightmoor", title: "Product Designer", phone: "+1 347-555-0123", circle: Colleague },
    Person { key: "lena", first: "Lena", last: "Fischer", email: "lena@brightmoor.co", org: "Brightmoor", title: "Head of Marketing", phone: "+1 646-555-0190", circle: Colleague },
    Person { key: "sam", first: "Sam", last: "Whitaker", email: "sam@brightmoor.co", org: "Brightmoor", title: "People & Operations", phone: "+1 212-555-0135", circle: Colleague },
    Person { key: "jordan", first: "Jordan", last: "Ellis", email: "jordan@brightmoor.co", org: "Brightmoor", title: "Account Executive", phone: "+1 718-555-0164", circle: Colleague },
    Person { key: "hiro", first: "Hiro", last: "Tanaka", email: "hiro@brightmoor.co", org: "Brightmoor", title: "Senior Engineer", phone: "+1 929-555-0112", circle: Colleague },
    Person { key: "chloe", first: "Chloé", last: "Martin", email: "chloe@brightmoor.co", org: "Brightmoor", title: "Customer Success", phone: "+1 646-555-0158", circle: Colleague },
    Person { key: "marcus", first: "Marcus", last: "Hill", email: "marcus@brightmoor.co", org: "Brightmoor", title: "Finance", phone: "+1 212-555-0186", circle: Colleague },
    Person { key: "ines", first: "Inês", last: "Duarte", email: "ines@brightmoor.co", org: "Brightmoor", title: "Data Analyst", phone: "+1 347-555-0199", circle: Colleague },
    Person { key: "oliver", first: "Oliver", last: "Grant", email: "oliver@brightmoor.co", org: "Brightmoor", title: "Engineer", phone: "+1 929-555-0141", circle: Colleague },
    Person { key: "zara", first: "Zara", last: "Ahmed", email: "zara@brightmoor.co", org: "Brightmoor", title: "Recruiting", phone: "+1 718-555-0107", circle: Colleague },
    Person { key: "ruth", first: "Ruth", last: "Abernathy", email: "ruth@brightmoor.co", org: "Brightmoor", title: "Office Manager", phone: "+1 212-555-0172", circle: Colleague },
    // Customers, partners, investors, vendors
    Person { key: "rachel", first: "Rachel", last: "Kim", email: "rachel.kim@callowayfreight.com", org: "Calloway Freight", title: "VP Operations", phone: "+1 312-555-0144", circle: Work },
    Person { key: "dev", first: "Dev", last: "Malhotra", email: "dev@callowayfreight.com", org: "Calloway Freight", title: "IT Director", phone: "+1 312-555-0119", circle: Work },
    Person { key: "grace", first: "Grace", last: "Liu", email: "grace@juniperpartners.vc", org: "Juniper Partners", title: "Partner", phone: "+1 415-555-0133", circle: Work },
    Person { key: "ben", first: "Ben", last: "Ortiz", email: "bortiz@marlowfinch.law", org: "Marlow & Finch LLP", title: "Associate", phone: "+1 212-555-0166", circle: Work },
    Person { key: "felix", first: "Felix", last: "Novak", email: "felix@novakstudio.design", org: "Novak Studio", title: "Illustrator", phone: "+420 555 018 220", circle: Work },
    Person { key: "amara", first: "Amara", last: "Nwosu", email: "amara@quillandoak.com", org: "Quill & Oak", title: "Founder", phone: "+1 503-555-0181", circle: Work },
    Person { key: "theo", first: "Theo", last: "Lindqvist", email: "theo@harborlightcoffee.com", org: "Harborlight Coffee", title: "Head of Retail", phone: "+1 206-555-0127", circle: Work },
    Person { key: "nadia", first: "Nadia", last: "Petrova", email: "nadia@ridgelinehealth.org", org: "Ridgeline Health", title: "Product Manager", phone: "+1 617-555-0152", circle: Work },
    Person { key: "carlos", first: "Carlos", last: "Mendes", email: "carlos@pemberton-cpa.com", org: "Pemberton CPA", title: "Senior Accountant", phone: "+1 212-555-0198", circle: Work },
    // Friends and family
    Person { key: "ngozi", first: "Ngozi", last: "Okafor", email: "ngozi.okafor@gmail.com", org: "", title: "", phone: "+1 301-555-0116", circle: Personal },
    Person { key: "emeka", first: "Emeka", last: "Okafor", email: "emekao@gmail.com", org: "", title: "", phone: "+1 240-555-0175", circle: Personal },
    Person { key: "jess", first: "Jess", last: "Moreno", email: "jessmoreno.nyc@gmail.com", org: "", title: "", phone: "+1 917-555-0189", circle: Personal },
    Person { key: "liam", first: "Liam", last: "O'Connell", email: "liam.oconnell@fastmail.com", org: "", title: "", phone: "+1 646-555-0120", circle: Personal },
    Person { key: "hannah", first: "Hannah", last: "Weiss", email: "hweiss@gmail.com", org: "", title: "", phone: "+1 718-555-0153", circle: Personal },
    // Other contacts
    Person { key: "yusuf", first: "Yusuf", last: "Karaca", email: "yusuf@tinkerform.io", org: "Tinkerform", title: "Founder", phone: "", circle: Acquaintance },
    Person { key: "mei", first: "Mei", last: "Watanabe", email: "mei.watanabe@hollisconf.org", org: "Hollis Conference", title: "Program Chair", phone: "", circle: Acquaintance },
    Person { key: "paul", first: "Paul", last: "Durand", email: "pdurand@westbrook-realty.com", org: "Westbrook Realty", title: "Property Manager", phone: "", circle: Acquaintance },
    Person { key: "sofia", first: "Sofia", last: "Romano", email: "sofia@romanocatering.com", org: "Romano Catering", title: "Owner", phone: "", circle: Acquaintance },
];

/// The account holder.
pub fn me() -> &'static Person {
    &PEOPLE[0]
}

/// A person by script handle. Panics on unknown handles: scripts are static data.
pub fn get(key: &str) -> &'static Person {
    PEOPLE
        .iter()
        .find(|person| person.key == key)
        .unwrap_or_else(|| panic!("fixture script names unknown person {key:?}"))
}
