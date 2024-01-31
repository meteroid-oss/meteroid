--: ProductFamily()

--! create_product_family () : ProductFamily
INSERT INTO
    product_family(id, name, external_id, tenant_id)
VALUES
    (:id, :name, :external_id, :tenant_id) RETURNING id,
    name,
    external_id;

--! list_product_families () : ProductFamily
SELECT
    id,
    name,
    external_id
FROM
    product_family
WHERE
    tenant_id = :tenant_id;

--! get_product_family_by_external_id () : ProductFamily
SELECT
    id,
    name,
    external_id
FROM
    product_family
WHERE
    tenant_id = :tenant_id
    AND external_id = :external_id;

--: Product(description?)
--! upsert_product (description?) : Product
INSERT INTO
    product (
        id,
        name,
        description,
        product_family_id,
        tenant_id,
        created_by
    )
VALUES
    (
        :id,
        :name,
        :description,
        (
            SELECT
                id
            FROM
                product_family
            WHERE
                external_id = :product_family_external_id
                AND tenant_id = :tenant_id
        ),
        :tenant_id,
        :created_by
    ) ON CONFLICT (id) DO
UPDATE
SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    created_by = EXCLUDED.created_by,
    product_family_id = EXCLUDED.product_family_id,
    tenant_id = EXCLUDED.tenant_id 
    RETURNING id, name, description,  created_at
    ;

--: ListProduct()
--! list_products () : ListProduct
SELECT
    p.id,
    p.name,
    count(*) OVER() AS total_count
FROM
    product AS p
    JOIN product_family AS pf ON pf.id = p.product_family_id
WHERE
    p.tenant_id = :tenant_id
    AND pf.external_id = :family_external_id
LIMIT
    :limit OFFSET :offset;

--! search_products (query?) : ListProduct
SELECT
    p.id,
    p.name,
    count(*) OVER() AS total_count
FROM
    product AS p
    JOIN product_family AS pf ON pf.id = p.product_family_id
WHERE
    p.tenant_id = :tenant_id
    AND pf.external_id = :family_external_id
    AND p.name ILIKE '%' || :query || '%'
LIMIT
    :limit OFFSET :offset;

--! get_product_details () : Product
SELECT
    p.id,
    p.name,
    p.description,
    p.created_at
FROM
    product AS p
WHERE
    p.id = :product_id
    AND p.tenant_id = :tenant_id;
