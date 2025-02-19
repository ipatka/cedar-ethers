/*
 * Copyright 2022-2023 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use super::{
    EntityUID, LinkingError, LiteralPolicy, Policy, PolicyID, ReificationError, SlotId,
    StaticPolicy, Template,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::Entry, HashMap};
use std::{borrow::Borrow, sync::Arc};
use thiserror::Error;

/// Represents a set of `Policy`s
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LiteralPolicySet")]
#[serde(into = "LiteralPolicySet")]
pub struct PolicySet {
    /// `templates` contains all bodies of policies in the `PolicySet`
    /// A body is either:
    ///    A Body of a `Template`, which has slots that need to be filled in
    ///    A Body of an `StaticPolicy`, which has been converted into a `Template` that has zero slots
    templates: HashMap<PolicyID, Arc<Template>>,
    /// `links` contains all of the executable policies in the `PolicySet`
    /// A `StaticPolicy` must have exactly one `Policy` in `links`
    ///   (this is managed by `PolicySet::add)
    /// A `Template` may have zero or many links
    links: HashMap<PolicyID, Policy>,
}

/// Converts a LiteralPolicySet into a PolicySet, ensuring the invariants are met
/// Every `Policy` must point to a `Template` that exists in the set.
impl TryFrom<LiteralPolicySet> for PolicySet {
    type Error = ReificationError;
    fn try_from(pset: LiteralPolicySet) -> Result<Self, Self::Error> {
        // Allocate the templates into Arc's
        let templates = pset
            .templates
            .into_iter()
            .map(|(id, template)| (id, Arc::new(template)))
            .collect();
        let links = pset
            .links
            .into_iter()
            .map(|(id, literal)| literal.reify(&templates).map(|linked| (id, linked)))
            .collect::<Result<HashMap<PolicyID, Policy>, ReificationError>>()?;
        Ok(Self { templates, links })
    }
}

/// A Policy Set that can be serialized, but does not maintain the invariants that `PolicySet` does
#[derive(Debug, Serialize, Deserialize)]
struct LiteralPolicySet {
    templates: HashMap<PolicyID, Template>,
    links: HashMap<PolicyID, LiteralPolicy>,
}

impl From<PolicySet> for LiteralPolicySet {
    fn from(pset: PolicySet) -> Self {
        let templates = pset
            .templates
            .into_iter()
            .map(|(id, template)| (id, template.as_ref().clone()))
            .collect();
        let links = pset
            .links
            .into_iter()
            .map(|(id, p)| (id, p.into()))
            .collect();
        Self { templates, links }
    }
}

/// Potential errors when working with `PolicySet`s.
#[derive(Error, Debug)]
pub enum PolicySetError {
    /// There was a duplicate [`PolicyID`] encountered in either the set of
    /// templates or the set of policies.
    #[error("duplicate template or policy id `{id}`")]
    Occupied {
        /// [`PolicyID`] that was duplicate
        id: PolicyID,
    },
}

// The public interface of `PolicySet` is intentionally narrow, to allow us
// maximum flexibility to change the underlying implementation in the future
impl PolicySet {
    /// Create a fresh empty `PolicySet`
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
            links: HashMap::new(),
        }
    }

    /// Add a `Policy` to the `PolicySet`.
    pub fn add(&mut self, policy: Policy) -> Result<(), PolicySetError> {
        let t = policy.template_arc();

        // we need to check for all possible errors before making any
        // modifications to `self`.
        // So we just collect the `ventry` here, and we only do the insertion
        // once we know there will be no error
        let template_ventry = match self.templates.entry(t.id().clone()) {
            Entry::Vacant(ventry) => Some(ventry),
            Entry::Occupied(oentry) => {
                if oentry.get() != &t {
                    return Err(PolicySetError::Occupied {
                        id: oentry.key().clone(),
                    });
                }
                None
            }
        };

        let link_ventry = match self.links.entry(policy.id().clone()) {
            Entry::Vacant(ventry) => Some(ventry),
            Entry::Occupied(oentry) => {
                return Err(PolicySetError::Occupied {
                    id: oentry.key().clone(),
                });
            }
        };

        // if we get here, there will be no errors.  So actually do the
        // insertions.
        if let Some(ventry) = template_ventry {
            ventry.insert(t);
        }
        if let Some(ventry) = link_ventry {
            ventry.insert(policy);
        }

        Ok(())
    }

    /// Add a `StaticPolicy` to the `PolicySet`.
    pub fn add_static(&mut self, policy: StaticPolicy) -> Result<(), PolicySetError> {
        let (t, p) = Template::link_static_policy(policy);

        // TODO: Use `try_insert` when stabilized.
        // https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.try_insert
        match (
            self.templates.entry(t.id().clone()),
            self.links.entry(t.id().clone()),
        ) {
            (Entry::Vacant(templates_entry), Entry::Vacant(links_entry)) => {
                templates_entry.insert(t);
                links_entry.insert(p);
                Ok(())
            }
            (Entry::Occupied(oentry), _) => Err(PolicySetError::Occupied {
                id: oentry.key().clone(),
            }),
            (_, Entry::Occupied(oentry)) => Err(PolicySetError::Occupied {
                id: oentry.key().clone(),
            }),
        }
    }

    /// Add a template to the policy set.
    pub fn add_template(&mut self, t: Template) -> Result<(), PolicySetError> {
        // TODO: Use `try_insert` when stabilized.
        // https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.try_insert
        match self.templates.entry(t.id().clone()) {
            Entry::Occupied(oentry) => Err(PolicySetError::Occupied {
                id: oentry.key().clone(),
            }),
            Entry::Vacant(ventry) => {
                ventry.insert(Arc::new(t));
                Ok(())
            }
        }
    }

    /// Attempt to create a new template linked policy and add it to the policy
    /// set. Returns a references to the new template linked policy if
    /// successful.
    ///
    /// Errors for two reasons
    ///   1) The the passed SlotEnv either does not match the slots in the templates
    ///   2) The passed link Id conflicts with an Id already in the set
    pub fn link(
        &mut self,
        template_id: PolicyID,
        new_id: PolicyID,
        values: HashMap<SlotId, EntityUID>,
    ) -> Result<&Policy, LinkingError> {
        let t = self
            .get_template(&template_id)
            .ok_or_else(|| LinkingError::NoSuchTemplate {
                id: template_id.clone(),
            })?;
        let r = Template::link(t, new_id.clone(), values)?;

        // Both maps must not contain the `new_id`
        match (
            self.links.entry(new_id.clone()),
            self.templates.entry(new_id),
        ) {
            (Entry::Vacant(links_entry), Entry::Vacant(_)) => Ok(links_entry.insert(r)),
            (Entry::Occupied(oentry), _) => Err(LinkingError::PolicyIdConflict {
                id: oentry.key().clone(),
            }),
            (_, Entry::Occupied(oentry)) => Err(LinkingError::PolicyIdConflict {
                id: oentry.key().clone(),
            }),
        }
    }

    /// Iterate over all policies
    pub fn policies(&self) -> impl Iterator<Item = &Policy> {
        self.links.values()
    }

    /// Iterate over everything stored as template, including static policies.
    /// Ie: all_templates() should equal templates() ++ static_policies().map(|p| p.template())
    pub fn all_templates(&self) -> impl Iterator<Item = &Template> {
        self.templates.values().map(|t| t.borrow())
    }

    /// Iterate over templates with slots
    pub fn templates(&self) -> impl Iterator<Item = &Template> {
        self.all_templates().filter(|t| t.slots().count() != 0)
    }

    /// Iterate over all of the static policies.
    pub fn static_policies(&self) -> impl Iterator<Item = &Policy> {
        self.policies().filter(|p| p.is_static())
    }

    /// Returns true iff the `PolicySet` is empty
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty() && self.links.is_empty()
    }

    /// Lookup a template by policy id
    pub fn get_template(&self, id: &PolicyID) -> Option<Arc<Template>> {
        self.templates.get(id).map(Arc::clone)
    }

    /// Lookup an policy by policy id
    pub fn get(&self, id: &PolicyID) -> Option<&Policy> {
        self.links.get(id)
    }

    /// Attempt to collect an iterator over policies into a PolicySet
    pub fn try_from_iter<T: IntoIterator<Item = Policy>>(iter: T) -> Result<Self, PolicySetError> {
        let mut set = Self::new();
        for p in iter {
            set.add(p)?;
        }
        Ok(set)
    }
}

impl std::fmt::Display for PolicySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // we don't show the ID, because the Display impl for Policy itself shows the ID
        if self.is_empty() {
            write!(f, "<empty policyset>")
        } else {
            write!(
                f,
                "Templates:\n{}, Template Linked Policies:\n{}",
                self.all_templates().join("\n"),
                self.policies().join("\n")
            )
        }
    }
}

// PANIC SAFETY tests
#[allow(clippy::indexing_slicing)]
#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, HashMap};

    use crate::{
        ast::{ActionConstraint, Effect, Expr, PrincipalConstraint, ResourceConstraint},
        parser,
    };

    use super::*;

    #[test]
    fn link_conflicts() {
        let mut pset = PolicySet::new();
        let p1 = parser::parse_policy(Some("id".into()), "permit(principal,action,resource);")
            .expect("Failed to parse");
        pset.add_static(p1).expect("Failed to add!");
        let template = parser::parse_policy_template(
            Some("t".into()),
            "permit(principal == ?principal, action, resource);",
        )
        .expect("Failed to parse");
        pset.add_template(template).expect("Add failed");

        let env: HashMap<SlotId, EntityUID> = [(
            SlotId::principal(),
            r#"Test::"test""#.parse().expect("Failed to parse"),
        )]
        .into_iter()
        .collect();

        let r = pset.link(PolicyID::from_string("t"), PolicyID::from_string("id"), env);

        match r {
            Ok(_) => panic!("Should have failed due to conflict"),
            Err(LinkingError::PolicyIdConflict { id }) => {
                assert_eq!(id, PolicyID::from_string("id"))
            }
            Err(e) => panic!("Incorrect error: {e}"),
        };
    }

    /// This test focuses on `PolicySet::add()`, while other tests mostly use
    /// `PolicySet::add_static()` and `PolicySet::link()`.
    #[test]
    fn policyset_add() {
        let mut pset = PolicySet::new();
        let static_policy =
            parser::parse_policy(Some("id".into()), "permit(principal,action,resource);")
                .expect("Failed to parse");
        let static_policy: Policy = static_policy.into();
        pset.add(static_policy)
            .expect("Adding static policy in Policy form should succeed");

        let template = Arc::new(
            parser::parse_policy_template(
                Some("t".into()),
                "permit(principal == ?principal, action, resource);",
            )
            .expect("Failed to parse"),
        );
        let env1: HashMap<SlotId, EntityUID> = [(
            SlotId::principal(),
            r#"Test::"test1""#.parse().expect("Failed to parse"),
        )]
        .into_iter()
        .collect();

        let p1 = Template::link(Arc::clone(&template), PolicyID::from_string("link"), env1)
            .expect("Failed to link");
        pset.add(p1).expect(
            "Adding link should succeed, even though the template wasn't previously in the pset",
        );
        assert!(
            pset.get_template(&PolicyID::from_string("t")).is_some(),
            "Adding link should implicitly add the template"
        );

        let env2: HashMap<SlotId, EntityUID> = [(
            SlotId::principal(),
            r#"Test::"test2""#.parse().expect("Failed to parse"),
        )]
        .into_iter()
        .collect();

        let p2 = Template::link(
            Arc::clone(&template),
            PolicyID::from_string("link"),
            env2.clone(),
        )
        .expect("Failed to link");
        match pset.add(p2) {
            Ok(_) => panic!("Should have failed due to conflict with existing link id"),
            Err(PolicySetError::Occupied { id }) => assert_eq!(id, PolicyID::from_string("link")),
        }

        let p3 = Template::link(Arc::clone(&template), PolicyID::from_string("link2"), env2)
            .expect("Failed to link");
        pset.add(p3).expect(
            "Adding link should succeed, even though the template already existed in the pset",
        );

        let template2 = Arc::new(
            parser::parse_policy_template(
                Some("t".into()),
                "forbid(principal, action, resource == ?resource);",
            )
            .expect("Failed to parse"),
        );
        let env3: HashMap<SlotId, EntityUID> = [(
            SlotId::resource(),
            r#"Test::"test3""#.parse().expect("Failed to parse"),
        )]
        .into_iter()
        .collect();

        let p4 = Template::link(
            Arc::clone(&template2),
            PolicyID::from_string("unique3"),
            env3,
        )
        .expect("Failed to link");
        match pset.add(p4) {
            Ok(_) => panic!("Should have failed due to conflict on template id"),
            Err(PolicySetError::Occupied { id }) => {
                assert_eq!(id, PolicyID::from_string("t"))
            }
        }
    }

    #[test]
    fn policy_conflicts() {
        let mut pset = PolicySet::new();
        let p1 = parser::parse_policy(Some("id".into()), "permit(principal,action,resource);")
            .expect("Failed to parse");
        let p2 = parser::parse_policy(
            Some("id".into()),
            "permit(principal,action,resource) when { false };",
        )
        .expect("Failed to parse");
        pset.add_static(p1).expect("Failed to add!");
        match pset.add_static(p2) {
            Ok(_) => panic!("Should have failed to due name conflict"),
            Err(PolicySetError::Occupied { id }) => assert_eq!(id, PolicyID::from_string("id")),
        }
    }

    #[test]
    fn template_filtering() {
        let template = parser::parse_policy_template(
            Some("template".into()),
            "permit(principal == ?principal, action, resource);",
        )
        .expect("Template Parse Failure");
        let static_policy = parser::parse_policy(
            Some("static".into()),
            "permit(principal, action, resource);",
        )
        .expect("Static parse failure");
        let mut set = PolicySet::new();
        set.add_template(template).unwrap();
        set.add_static(static_policy).unwrap();

        assert_eq!(set.all_templates().count(), 2);
        assert_eq!(set.templates().count(), 1);
        assert_eq!(set.static_policies().count(), 1);
        assert_eq!(set.policies().count(), 1);
        set.link(
            PolicyID::from_string("template"),
            PolicyID::from_string("id"),
            [(SlotId::principal(), EntityUID::with_eid("eid"))]
                .into_iter()
                .collect(),
        )
        .expect("Linking failed!");
        assert_eq!(set.static_policies().count(), 1);
        assert_eq!(set.policies().count(), 2);
    }

    #[test]
    fn linking_missing_template() {
        let tid = PolicyID::from_string("template");
        let lid = PolicyID::from_string("link");
        let t = Template::new(
            tid.clone(),
            BTreeMap::new(),
            Effect::Permit,
            PrincipalConstraint::any(),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        );

        let mut s = PolicySet::new();
        let e = s
            .link(tid.clone(), lid.clone(), HashMap::new())
            .expect_err("Should fail");

        match e {
            LinkingError::NoSuchTemplate { id } => assert_eq!(tid, id),
            e => panic!("Wrong error {e}"),
        };

        s.add_template(t).unwrap();
        s.link(tid, lid, HashMap::new()).expect("Should succeed");
    }

    #[test]
    fn linkinv_valid_link() {
        let tid = PolicyID::from_string("template");
        let lid = PolicyID::from_string("link");
        let t = Template::new(
            tid.clone(),
            BTreeMap::new(),
            Effect::Permit,
            PrincipalConstraint::is_eq_slot(),
            ActionConstraint::any(),
            ResourceConstraint::is_in_slot(),
            Expr::val(true),
        );

        let mut s = PolicySet::new();
        s.add_template(t).unwrap();

        let mut vals = HashMap::new();
        vals.insert(SlotId::principal(), EntityUID::with_eid("p"));
        vals.insert(SlotId::resource(), EntityUID::with_eid("a"));

        s.link(tid.clone(), lid.clone(), vals).expect("Should link");

        let v: Vec<_> = s.policies().collect();

        assert_eq!(v[0].id(), &lid);
        assert_eq!(v[0].template().id(), &tid);
    }

    #[test]
    fn linking_empty_set() {
        let s = PolicySet::new();
        assert_eq!(s.policies().count(), 0);
    }

    #[test]
    fn linking_raw_policy() {
        let mut s = PolicySet::new();
        let id = PolicyID::from_string("id");
        let p = StaticPolicy::new(
            id.clone(),
            BTreeMap::new(),
            Effect::Forbid,
            PrincipalConstraint::any(),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        )
        .expect("Policy Creation Failed");
        s.add_static(p).unwrap();

        let mut iter = s.policies();
        match iter.next() {
            Some(pol) => {
                assert_eq!(pol.id(), &id);
                assert_eq!(pol.effect(), Effect::Forbid);
                assert!(pol.env().is_empty())
            }
            None => panic!("Linked Record Not Present"),
        };
    }

    #[test]
    fn link_slotmap() {
        let mut s = PolicySet::new();
        let template_id = PolicyID::from_string("template");
        let link_id = PolicyID::from_string("link");
        let t = Template::new(
            template_id.clone(),
            BTreeMap::new(),
            Effect::Forbid,
            PrincipalConstraint::is_eq_slot(),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        );
        s.add_template(t).unwrap();

        let mut v = HashMap::new();
        let entity = EntityUID::with_eid("eid");
        v.insert(SlotId::principal(), entity.clone());
        s.link(template_id.clone(), link_id.clone(), v)
            .expect("Linking failed!");

        let link = s.get(&link_id).expect("Link should exist");
        assert_eq!(&link_id, link.id());
        assert_eq!(&template_id, link.template().id());
        assert_eq!(
            &entity,
            link.env()
                .get(&SlotId::principal())
                .expect("Mapping was incorrect")
        );
    }

    #[test]
    fn policy_sets() {
        let mut pset = PolicySet::new();
        assert!(pset.is_empty());
        let id1 = PolicyID::from_string("id1");
        let tid1 = PolicyID::from_string("template");
        let policy1 = StaticPolicy::new(
            id1.clone(),
            BTreeMap::new(),
            Effect::Permit,
            PrincipalConstraint::any(),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        )
        .expect("Policy Creation Failed");
        let template1 = Template::new(
            tid1.clone(),
            BTreeMap::new(),
            Effect::Permit,
            PrincipalConstraint::any(),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        );
        let added = pset.add_static(policy1.clone()).is_ok();
        assert!(added);
        let added = pset.add_static(policy1).is_ok();
        assert!(!added);
        let added = pset.add_template(template1.clone()).is_ok();
        assert!(added);
        let added = pset.add_template(template1).is_ok();
        assert!(!added);
        assert!(!pset.is_empty());
        let id2 = PolicyID::from_string("id2");
        let policy2 = StaticPolicy::new(
            id2.clone(),
            BTreeMap::new(),
            Effect::Forbid,
            PrincipalConstraint::is_eq(EntityUID::with_eid("jane")),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        )
        .expect("Policy Creation Failed");
        let added = pset.add_static(policy2).is_ok();
        assert!(added);

        let tid2 = PolicyID::from_string("template2");
        let template2 = Template::new(
            tid2.clone(),
            BTreeMap::new(),
            Effect::Permit,
            PrincipalConstraint::is_eq_slot(),
            ActionConstraint::any(),
            ResourceConstraint::any(),
            Expr::val(true),
        );
        let id3 = PolicyID::from_string("link");
        let added = pset.add_template(template2).is_ok();
        assert!(added);

        let r = pset.link(
            tid2.clone(),
            id3.clone(),
            HashMap::from([(SlotId::principal(), EntityUID::with_eid("example"))]),
        );
        r.expect("Linking failed");

        assert_eq!(pset.get(&id1).expect("should find the policy").id(), &id1);
        assert_eq!(pset.get(&id2).expect("should find the policy").id(), &id2);
        assert_eq!(pset.get(&id3).expect("should find link").id(), &id3);
        assert_eq!(
            pset.get(&id3).expect("should find link").template().id(),
            &tid2
        );
        assert!(pset.get(&tid2).is_none());
        assert!(pset.get_template(&id1).is_some()); // Static policies are also templates
        assert!(pset.get_template(&id2).is_some()); // Static policies are also templates
        assert!(pset.get_template(&tid2).is_some());
        assert_eq!(pset.policies().count(), 3);

        assert_eq!(
            pset.get_template(&tid1)
                .expect("should find the template")
                .id(),
            &tid1
        );
        assert!(pset.get(&tid1).is_none());
        assert_eq!(pset.all_templates().count(), 4);
    }
}
