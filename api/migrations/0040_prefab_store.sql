-- Store de prefabs (ADR-0042) : une version peut venir d'un store (index signé d'un dépôt de
-- prefabs, archive vérifiée par sa somme). `store` nomme le store configuré d'où elle vient.
ALTER TABLE prefabs
    DROP CONSTRAINT prefabs_source_check,
    ADD CONSTRAINT prefabs_source_check CHECK (source IN ('official', 'uploaded', 'trial', 'store')),
    ADD COLUMN store text,
    ADD CONSTRAINT prefabs_store_check CHECK ((source = 'store') = (store IS NOT NULL));
